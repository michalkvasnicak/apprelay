import Foundation
import Network
import Security

struct VpnSettings {
    let host: String
    let port: Int
    let publicKey: String
    let serverAddress: String
    let clientAddress: String

    init(_ values: [String: Any]) throws {
        guard let host = values["host"] as? String,
              let port = values["port"] as? Int, (1...65535).contains(port),
              let publicKey = values["publicKey"] as? String,
              let keyData = Data(base64Encoded: publicKey), keyData.count == 32,
              keyData.contains(where: { $0 != 0 }),
              let serverAddress = values["serverAddress"] as? String,
              let clientAddress = values["clientAddress"] as? String,
              Self.isUnicastIPv4(host), Self.isUnicastIPv4(serverAddress),
              Self.isUnicastIPv4(clientAddress),
              host != serverAddress, host != clientAddress, serverAddress != clientAddress else {
            throw VpnError.invalidConfiguration
        }
        self.host = host
        self.port = port
        self.publicKey = publicKey
        self.serverAddress = serverAddress
        self.clientAddress = clientAddress
    }

    var dictionary: [String: Any] {
        ["host": host, "port": port, "publicKey": publicKey,
         "serverAddress": serverAddress, "clientAddress": clientAddress]
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
            return "Invalid VPN settings. Use distinct endpoint, server and device IPv4 addresses, a valid port and server public key."
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
