import CryptoKit
import Foundation
import Security

// Shared by the containing app and extension. The WireGuard private key stays native.
enum VpnKeychain {
    struct Identity {
        let publicKey: String
        let reference: Data
    }

    private enum Account: String {
        case privateKey = "device-private-key"
        case token = "device-registration-token"
    }

    private static let service = "AppRelay.Device"

    private static func serviceQuery(_ service: String) throws -> [String: Any] {
        // The bundle-specific access group keeps local and production credentials separate.
        guard let group = Bundle.main.object(forInfoDictionaryKey: "AppRelayKeychainGroup") as? String,
              !group.isEmpty, !group.contains("$(") else { throw VpnError.missingKeychainGroup }
        return [kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service,
                kSecAttrAccessGroup as String: group]
    }

    private static func query(_ account: Account) throws -> [String: Any] {
        var request = try serviceQuery(service)
        request[kSecAttrAccount as String] = account.rawValue
        return request
    }

    // Missing or invalid key bytes mean there is no usable identity. Access failures throw.
    static func identity() throws -> Identity? {
        var request = try query(.privateKey)
        request[kSecReturnData as String] = true
        request[kSecReturnPersistentRef as String] = true
        guard let result = try copy(request) else { return nil }
        guard let item = result as? [String: Any],
              let data = item[kSecValueData as String] as? Data,
              let reference = item[kSecValuePersistentRef as String] as? Data else {
            throw VpnError.keychain(errSecDecode)
        }
        guard let key = try? Curve25519.KeyAgreement.PrivateKey(rawRepresentation: data) else { return nil }
        return Identity(publicKey: key.publicKey.rawRepresentation.base64EncodedString(), reference: reference)
    }

    static func createIdentity() throws -> Identity {
        try add(Curve25519.KeyAgreement.PrivateKey().rawRepresentation, to: query(.privateKey))
        guard let identity = try identity() else { throw VpnError.missingKey }
        return identity
    }

    static func registrationToken() throws -> String? {
        var request = try query(.token)
        request[kSecReturnData as String] = true
        guard let result = try copy(request) else { return nil }
        guard let data = result as? Data, let token = String(data: data, encoding: .utf8), !token.isEmpty else {
            throw VpnError.keychain(errSecDecode)
        }
        return token
    }

    static func saveRegistrationToken(_ token: String) throws {
        let request = try query(.token)
        let data = Data(token.utf8)
        let status = SecItemUpdate(request as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            try add(data, to: request)
        } else if status != errSecSuccess {
            throw VpnError.keychain(status)
        }
    }

    static func clearCredentials() throws {
        try delete(query(.token))
        try delete(query(.privateKey))
    }

    static func discardPreviousStorage() throws {
        // Clean-break upgrade: delete this app's old VPN entries without reading any of them.
        try delete(serviceQuery("AppRelay.WireGuard"))
    }

    static func privateKey(reference: Data) throws -> Data {
        // The reference identifies the item; the extension's entitlements authorize access.
        guard let result = try copy([kSecValuePersistentRef as String: reference,
                                     kSecReturnData as String: true]) else { throw VpnError.missingKey }
        guard let data = result as? Data, data.count == 32 else { throw VpnError.keychain(errSecDecode) }
        return data
    }

    private static func copy(_ request: [String: Any]) throws -> CFTypeRef? {
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw VpnError.keychain(status) }
        guard let result else { throw VpnError.keychain(errSecDecode) }
        return result
    }

    private static func add(_ data: Data, to request: [String: Any]) throws {
        var item = request
        item[kSecValueData as String] = data
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else { throw VpnError.keychain(status) }
    }

    private static func delete(_ request: [String: Any]) throws {
        let status = SecItemDelete(request as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw VpnError.keychain(status)
        }
    }
}
