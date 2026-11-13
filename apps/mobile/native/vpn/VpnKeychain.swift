import CryptoKit
import Foundation
import Security

// Shared by the containing app and extension. Private material never crosses the JS bridge.
enum VpnKeychain {
    private static func query() throws -> [String: Any] {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "AppRelayKeychainGroup") as? String,
              !group.isEmpty, !group.contains("$(") else { throw VpnError.missingKey }
        return [kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: "AppRelay.WireGuard",
                kSecAttrAccount as String: "device-private-key",
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
