import CryptoKit
import Foundation
import Security

// Shared by the containing app and extension. The WireGuard private key stays native.
enum VpnKeychain {
    private static func query(account: String = "device-private-key") throws -> [String: Any] {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "AppRelayKeychainGroup") as? String,
              !group.isEmpty, !group.contains("$(") else { throw VpnError.missingKey }
        return [kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: "AppRelay.WireGuard",
                kSecAttrAccount as String: account,
                kSecAttrAccessGroup as String: group]
    }

    static func identity() throws -> (publicKey: String, reference: Data) {
        var request = try query()
        request[kSecReturnData as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        let data: Data
        if status == errSecItemNotFound {
            data = Curve25519.KeyAgreement.PrivateKey().rawRepresentation
            var item = try query()
            item[kSecValueData as String] = data
            item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let added = SecItemAdd(item as CFDictionary, nil)
            guard added == errSecSuccess else { throw VpnError.keychain(added) }
        } else {
            guard status == errSecSuccess, let stored = result as? Data else {
                throw VpnError.keychain(status)
            }
            data = stored
        }
        let key = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: data)
        var referenceQuery = try query()
        referenceQuery[kSecReturnPersistentRef as String] = true
        var reference: CFTypeRef?
        let referenceStatus = SecItemCopyMatching(referenceQuery as CFDictionary, &reference)
        guard referenceStatus == errSecSuccess, let reference = reference as? Data else {
            throw VpnError.keychain(referenceStatus)
        }
        return (key.publicKey.rawRepresentation.base64EncodedString(), reference)
    }

    // The access group is derived from the app bundle ID, isolating local and production apps.
    private static let registrationAccount = "device-registration-token"

    static func registrationToken() throws -> String? {
        if let token = try token(account: registrationAccount) { return token }
        return try migrateRegistrationToken()
    }

    private static func token(account: String) throws -> String? {
        var request = try query(account: account)
        request[kSecReturnData as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data,
              let token = String(data: data, encoding: .utf8) else {
            throw VpnError.keychain(status)
        }
        return token
    }

    private static func migrateRegistrationToken() throws -> String? {
        let publicKey = try identity().publicKey
        var request = try query()
        request.removeValue(forKey: kSecAttrAccount as String)
        request[kSecReturnAttributes as String] = true
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitAll
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let items = result as? [[String: Any]] else {
            throw VpnError.keychain(status)
        }
        // Recover URL-scoped credentials for this identity only, within this app’s access group.
        var tokens = Set<String>()
        for item in items {
            guard let account = item[kSecAttrAccount as String] as? String,
                  account.hasPrefix("bearer:"), account.hasSuffix(":" + publicKey) else { continue }
            guard let data = item[kSecValueData as String] as? Data,
                  let token = String(data: data, encoding: .utf8) else {
                throw VpnError.keychain(errSecDecode)
            }
            tokens.insert(token)
        }
        guard tokens.count <= 1 else {
            throw NSError(domain: "AppRelayVPN", code: 1, userInfo: [
                NSLocalizedDescriptionKey: "Multiple saved device tokens were found; registration recovery is required."
            ])
        }
        guard let token = tokens.first else { return nil }
        try saveRegistrationToken(token: token)
        return token
    }

    static func saveRegistrationToken(token: String) throws {
        let request = try query(account: registrationAccount)
        let data = Data(token.utf8)
        let status = SecItemUpdate(request as CFDictionary,
                                   [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var item = request
            item[kSecValueData as String] = data
            item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let added = SecItemAdd(item as CFDictionary, nil)
            guard added == errSecSuccess else { throw VpnError.keychain(added) }
        } else if status != errSecSuccess {
            throw VpnError.keychain(status)
        }
    }

    static func resetRegistration() throws {
        // This service contains only the WireGuard identity and registration credentials.
        // Delete legacy token entries too, so migration cannot restore a rejected token.
        var request = try query()
        request.removeValue(forKey: kSecAttrAccount as String)
        let status = SecItemDelete(request as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw VpnError.keychain(status)
        }
    }

    static func privateKey(reference: Data) throws -> Data {
        // The persistent reference identifies the item; signed entitlements still
        // enforce access to its shared group, without relying on plist expansion.
        let request: [String: Any] = [kSecValuePersistentRef as String: reference,
                                      kSecReturnData as String: true]
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        guard status == errSecSuccess, let data = result as? Data, data.count == 32 else {
            throw VpnError.keychain(status)
        }
        return data
    }
}
