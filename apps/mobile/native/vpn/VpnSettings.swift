import Foundation
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
              let port = values["port"] as? Int,
              let publicKey = values["publicKey"] as? String,
              let clientAddress = values["clientAddress"] as? String,
              let allowedIPs = values["allowedIPs"] as? [String],
              let keepalive = values["persistentKeepaliveSeconds"] as? Int else {
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
            return "Unable to load the VPN configuration."
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
