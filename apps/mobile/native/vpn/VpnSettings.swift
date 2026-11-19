import Foundation
import Security

struct VpnSettings {
    let host: String
    let port: Int
    let publicKey: String
    let clientAddress: String
    let allowedIPs: [String]
    let persistentKeepaliveSeconds: Int
    let remotePairingReportURL: URL?
    let remotePairingAppKey: String?

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
        if let value = values["remotePairingReportURL"] as? String,
           let url = URL(string: value),
           url.scheme == "https", url.host != nil, url.user == nil, url.password == nil,
           let appKey = values["remotePairingAppKey"] as? String {
            self.remotePairingReportURL = url
            self.remotePairingAppKey = appKey
        } else {
            self.remotePairingReportURL = nil
            self.remotePairingAppKey = nil
        }
    }

    var dictionary: [String: Any] {
        var values: [String: Any] = [
            "host": host, "port": port, "publicKey": publicKey,
            "clientAddress": clientAddress, "allowedIPs": allowedIPs,
            "persistentKeepaliveSeconds": persistentKeepaliveSeconds,
        ]
        values["remotePairingReportURL"] = remotePairingReportURL?.absoluteString
        values["remotePairingAppKey"] = remotePairingAppKey
        return values
    }
}

enum VpnError: LocalizedError {
    case invalidConfiguration
    case keychain(OSStatus)
    case missingKeychainGroup
    case missingKey
    case busy
    case simulator
    case wifiRequired

    var errorDescription: String? {
        switch self {
        case .invalidConfiguration:
            return "Unable to load the VPN configuration."
        case .keychain(let status):
            return "Unable to access the VPN keychain (\(status)). Check signing and keychain-sharing entitlements."
        case .missingKeychainGroup:
            return "The VPN keychain access group is not configured."
        case .missingKey:
            return "The VPN device key is unavailable. Open the app and configure the VPN again."
        case .busy:
            return "A VPN operation is already in progress."
        case .simulator:
            return "WireGuard requires a signed build on a physical iPhone or iPad."
        case .wifiRequired:
            return "Connect to Wi-Fi to change the VPN connection."
        }
    }
}
