import Foundation
import Network
import NetworkExtension
import WireGuardKit

final class PacketTunnelProvider: NEPacketTunnelProvider {
    // Do not log WireGuard configuration or runtime output (it contains keys).
    private lazy var adapter = WireGuardAdapter(with: self) { _, _ in }
    private var allowedRoutes: [NEIPv4Route] = []

    override func startTunnel(options: [String: NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        do {
            guard let configuration = protocolConfiguration as? NETunnelProviderProtocol,
                  !configuration.includeAllNetworks,
                  let values = configuration.providerConfiguration,
                  let reference = configuration.passwordReference else { throw VpnError.invalidConfiguration }
            let settings = try VpnSettings(values)
            guard let privateKey = PrivateKey(rawValue: try VpnKeychain.privateKey(reference: reference)),
                  let publicKey = PublicKey(base64Key: settings.publicKey),
                  let clientAddress = IPAddressRange(from: settings.clientAddress),
                  let endpoint = Endpoint(from: "\(settings.host):\(settings.port)"),
                  let keepalive = UInt16(exactly: settings.persistentKeepaliveSeconds) else {
                throw VpnError.invalidConfiguration
            }
            var interface = InterfaceConfiguration(privateKey: privateKey)
            interface.addresses = [clientAddress]
            // No DNS override and no IPv6 configuration.
            var peer = PeerConfiguration(publicKey: publicKey)
            peer.endpoint = endpoint
            peer.allowedIPs = settings.allowedIPs.compactMap { IPAddressRange(from: $0) }
            guard peer.allowedIPs.count == settings.allowedIPs.count else { throw VpnError.invalidConfiguration }
            peer.persistentKeepAlive = keepalive
            allowedRoutes = try settings.allowedIPs.map { cidr in
                let parts = cidr.split(separator: "/", omittingEmptySubsequences: false)
                guard parts.count == 2, IPv4Address(String(parts[0])) != nil,
                      let prefix = Int(parts[1]), (0...32).contains(prefix) else {
                    throw VpnError.invalidConfiguration
                }
                let mask: UInt32 = prefix == 0 ? 0 : UInt32.max << (32 - prefix)
                let subnetMask = [24, 16, 8, 0].map { String((mask >> $0) & 255) }.joined(separator: ".")
                return NEIPv4Route(destinationAddress: String(parts[0]), subnetMask: subnetMask)
            }
            adapter.start(tunnelConfiguration: TunnelConfiguration(
                name: "AppRelay", interface: interface, peers: [peer]
            )) { error in
                guard let error else { completionHandler(nil); return }
                if case .setNetworkSettings(let underlying) = error {
                    completionHandler(underlying)
                } else {
                    completionHandler(NSError(domain: "AppRelay.WireGuard", code: 1,
                                              userInfo: [NSLocalizedDescriptionKey: "Unable to start the WireGuard backend."]))
                }
            }
        } catch { completionHandler(error) }
    }

    // Replace generated interface routes with the API's split routes, including
    // on network-path reconfiguration.
    override func setTunnelNetworkSettings(_ tunnelNetworkSettings: NETunnelNetworkSettings?,
                                          completionHandler: ((Error?) -> Void)? = nil) {
        guard let settings = tunnelNetworkSettings as? NEPacketTunnelNetworkSettings else {
            if tunnelNetworkSettings == nil {
                super.setTunnelNetworkSettings(nil, completionHandler: completionHandler)
            } else {
                completionHandler?(VpnError.invalidConfiguration)
            }
            return
        }
        guard !allowedRoutes.isEmpty, let ipv4 = settings.ipv4Settings else {
            completionHandler?(VpnError.invalidConfiguration)
            return
        }
        ipv4.includedRoutes = allowedRoutes
        ipv4.excludedRoutes = []
        settings.ipv6Settings = nil
        settings.dnsSettings = nil
        super.setTunnelNetworkSettings(settings, completionHandler: completionHandler)
    }

    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        adapter.stop { _ in completionHandler() }
    }
}
