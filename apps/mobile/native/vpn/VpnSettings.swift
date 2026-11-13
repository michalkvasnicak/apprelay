import Foundation
import Network
import Security

struct VpnSettings {
    let host: String
    let port: Int
    let publicKey: String
    let clientAddress: String
    let allowedIPs: [String]
    let persistentKeepaliveSeconds: Int

    init(_ values: [String: Any]) throws {
        guard let host = values["host"] as? String,
              let port = values["port"] as? Int, (1...65535).contains(port),
              let publicKey = values["publicKey"] as? String,
              let keyData = Data(base64Encoded: publicKey), keyData.count == 32,
              keyData.contains(where: { $0 != 0 }),
              let clientAddress = values["clientAddress"] as? String,
              let allowedIPs = values["allowedIPs"] as? [String], !allowedIPs.isEmpty,
              let keepalive = values["persistentKeepaliveSeconds"] as? Int,
              (0...65535).contains(keepalive),
              Self.isEndpointHost(host),
              clientAddress.hasSuffix("/32"),
              Self.isUnicastIPv4(String(clientAddress.dropLast(3))),
              allowedIPs.allSatisfy({ Self.isIPv4Route($0) }) else {
            throw VpnError.invalidConfiguration
        }
        self.host = host
        self.port = port
        self.publicKey = publicKey
        self.clientAddress = clientAddress
        self.allowedIPs = allowedIPs
        self.persistentKeepaliveSeconds = keepalive
    }

    var dictionary: [String: Any] {
        ["host": host, "port": port, "publicKey": publicKey,
         "clientAddress": clientAddress, "allowedIPs": allowedIPs,
         "persistentKeepaliveSeconds": persistentKeepaliveSeconds]
    }

    private static func isEndpointHost(_ value: String) -> Bool {
        if IPv4Address(value) != nil { return isUnicastIPv4(value) }
        guard value.count <= 253, value.contains(".") else { return false }
        return value.split(separator: ".", omittingEmptySubsequences: false).allSatisfy {
            $0.range(of: "^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$", options: .regularExpression) != nil
        }
    }

    // Keep the existing split-tunnel policy: IPv4 only, never a default route.
    private static func isIPv4Route(_ value: String) -> Bool {
        let parts = value.split(separator: "/", omittingEmptySubsequences: false)
        guard parts.count == 2, let prefix = Int(parts[1]), (1...32).contains(prefix) else { return false }
        return isUnicastIPv4(String(parts[0]))
    }

    private static func isUnicastIPv4(_ value: String) -> Bool {
        guard let address = IPv4Address(value), address.debugDescription == value,
              let first = address.rawValue.first else { return false }
        return first != 0 && first != 127 && first < 224
    }
}

enum VpnError: LocalizedError {
    case invalidConfiguration
    case keychain(OSStatus)
    case missingKey
    case busy
    case simulator

    var errorDescription: String? {
        switch self {
        case .invalidConfiguration:
            return "Invalid VPN settings. Expected a DNS/IPv4 endpoint, a device IPv4 /32, IPv4 split routes, valid keepalive, port and server public key."
        case .keychain(let status):
            return "Unable to access the VPN keychain (\(status)). Check signing and keychain-sharing entitlements."
        case .missingKey:
            return "The VPN device key is unavailable. Open the app and configure the VPN again."
        case .busy:
            return "A VPN operation is already in progress."
        case .simulator:
            return "WireGuard requires a signed build on a physical iPhone."
        }
    }
}
