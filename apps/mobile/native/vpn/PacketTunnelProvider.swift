import Foundation
import NetworkExtension
import WireGuardKit

final class PacketTunnelProvider: NEPacketTunnelProvider {
    // Do not log WireGuard configuration or runtime output (it contains keys).
    private lazy var adapter = WireGuardAdapter(with: self) { _, _ in }
    private var routedServerAddress: String?

    override func startTunnel(options: [String: NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        do {
            guard let configuration = protocolConfiguration as? NETunnelProviderProtocol,
                  !configuration.includeAllNetworks,
                  let values = configuration.providerConfiguration,
                  let reference = configuration.passwordReference else { throw VpnError.invalidConfiguration }
            let settings = try VpnSettings(values)
            guard let privateKey = PrivateKey(rawValue: try VpnKeychain.privateKey(reference: reference)),
                  let publicKey = PublicKey(base64Key: settings.publicKey),
                  let clientAddress = IPAddressRange(from: "\(settings.clientAddress)/32"),
                  let allowedIP = IPAddressRange(from: "\(settings.serverAddress)/32"),
                  let endpoint = Endpoint(from: "\(settings.host):\(settings.port)") else {
                throw VpnError.invalidConfiguration
            }
            var interface = InterfaceConfiguration(privateKey: privateKey)
            interface.addresses = [clientAddress]
            // No DNS override and no IPv6 configuration.
            var peer = PeerConfiguration(publicKey: publicKey)
            peer.endpoint = endpoint
            peer.allowedIPs = [allowedIP]
            peer.persistentKeepAlive = 25
            routedServerAddress = settings.serverAddress
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

    // WireGuardKit also generates an interface-address route. Replace its generated
    // routes with exactly the server /32, including on network-path reconfiguration.
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
        guard let server = routedServerAddress, let ipv4 = settings.ipv4Settings else {
            completionHandler?(VpnError.invalidConfiguration)
            return
        }
        ipv4.includedRoutes = [NEIPv4Route(destinationAddress: server, subnetMask: "255.255.255.255")]
        ipv4.excludedRoutes = []
        settings.ipv6Settings = nil
        settings.dnsSettings = nil
        super.setTunnelNetworkSettings(settings, completionHandler: completionHandler)
    }

    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        adapter.stop { _ in completionHandler() }
    }
}
