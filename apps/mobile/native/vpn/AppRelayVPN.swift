import Foundation
import Network
import NetworkExtension
import React

@objc(AppRelayVPN)
final class AppRelayVPN: NSObject {
    private var operationInProgress = false
    private var pairingDiscovery: NWBrowser?
    private var providerIdentifier: String { Bundle.main.bundleIdentifier! + ".PacketTunnel" }

    @objc static func requiresMainQueueSetup() -> Bool { true }

    @objc(getIdentity:rejecter:)
    func getIdentity(_ resolve: @escaping RCTPromiseResolveBlock, rejecter reject: @escaping RCTPromiseRejectBlock) {
        Task { @MainActor in
            do {
                let identity = try VpnKeychain.identity()
                resolve(["publicKey": identity.publicKey])
            } catch { reject("VPN_IDENTITY", error.localizedDescription, error) }
        }
    }

    @objc(getRegistrationToken:resolver:rejecter:)
    func getRegistrationToken(_ scope: String, resolver resolve: @escaping RCTPromiseResolveBlock,
                              rejecter reject: @escaping RCTPromiseRejectBlock) {
        do {
            resolve(try VpnKeychain.registrationToken(scope: scope))
        } catch { reject("VPN_CREDENTIAL", error.localizedDescription, error) }
    }

    @objc(saveRegistrationToken:token:resolver:rejecter:)
    func saveRegistrationToken(_ scope: String, token: String,
                               resolver resolve: @escaping RCTPromiseResolveBlock,
                               rejecter reject: @escaping RCTPromiseRejectBlock) {
        do {
            try VpnKeychain.saveRegistrationToken(scope: scope, token: token)
            resolve(nil)
        } catch { reject("VPN_CREDENTIAL", error.localizedDescription, error) }
    }

    @objc(getStatus:rejecter:)
    func getStatus(_ resolve: @escaping RCTPromiseResolveBlock, rejecter reject: @escaping RCTPromiseRejectBlock) {
        Task { @MainActor in
            do {
                let manager = try await loadManager()
                resolve(Self.status(manager?.connection.status ?? .disconnected))
            } catch { reject("VPN_STATUS", error.localizedDescription, error) }
        }
    }

    @objc(connect:resolver:rejecter:)
    func connect(_ values: [String: Any], resolver resolve: @escaping RCTPromiseResolveBlock,
                 rejecter reject: @escaping RCTPromiseRejectBlock) {
        Task { @MainActor in
            guard !operationInProgress else {
                reject("VPN_BUSY", VpnError.busy.localizedDescription, VpnError.busy)
                return
            }
            operationInProgress = true
            defer { operationInProgress = false }
            do {
                #if targetEnvironment(simulator)
                throw VpnError.simulator
                #else
                let settings = try VpnSettings(values)
                let identity = try VpnKeychain.identity()
                let manager = try await loadManager() ?? NETunnelProviderManager()
                guard [.invalid, .disconnected].contains(manager.connection.status) else {
                    throw VpnError.busy
                }
                let configuration = NETunnelProviderProtocol()
                configuration.providerBundleIdentifier = providerIdentifier
                configuration.serverAddress = settings.host
                configuration.providerConfiguration = settings.dictionary
                configuration.passwordReference = identity.reference
                configuration.includeAllNetworks = false
                configuration.enforceRoutes = false
                configuration.excludeLocalNetworks = false
                configuration.disconnectOnSleep = false
                manager.protocolConfiguration = configuration
                manager.localizedDescription = "AppRelay private server"
                manager.isEnabled = true
                manager.isOnDemandEnabled = false
                manager.onDemandRules = []
                try await save(manager)
                try await reload(manager)
                try manager.connection.startVPNTunnel()
                startPairingDiscovery()
                // This means startup was requested, not that the peer has handshaken.
                resolve(nil)
                #endif
            } catch { reject("VPN_CONNECT", error.localizedDescription, error) }
        }
    }

    @objc(preparePairing:rejecter:)
    func preparePairing(_ resolve: @escaping RCTPromiseResolveBlock, rejecter reject: @escaping RCTPromiseRejectBlock) {
        Task { @MainActor in
            startPairingDiscovery()
            resolve(nil)
        }
    }

    // Also needed on a cold launch that reuses an already-running tunnel.
    @MainActor private func startPairingDiscovery() {
        pairingDiscovery?.cancel()
        let browser = NWBrowser(for: .bonjour(type: "_remotepairing-pairable-host._tcp", domain: "local."),
                                using: .tcp)
        pairingDiscovery = browser
        browser.start(queue: .main)
    }

    @objc(disconnect:rejecter:)
    func disconnect(_ resolve: @escaping RCTPromiseResolveBlock, rejecter reject: @escaping RCTPromiseRejectBlock) {
        Task { @MainActor in
            guard !operationInProgress else {
                reject("VPN_BUSY", VpnError.busy.localizedDescription, VpnError.busy)
                return
            }
            operationInProgress = true
            defer { operationInProgress = false }
            do {
                let manager = try await loadManager()
                manager?.connection.stopVPNTunnel()
                pairingDiscovery?.cancel()
                pairingDiscovery = nil
                resolve(nil)
            } catch { reject("VPN_DISCONNECT", error.localizedDescription, error) }
        }
    }

    private func loadManager() async throws -> NETunnelProviderManager? {
        let managers: [NETunnelProviderManager] = try await withCheckedThrowingContinuation { continuation in
            NETunnelProviderManager.loadAllFromPreferences { managers, error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume(returning: managers ?? []) }
            }
        }
        return managers.first {
            ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == providerIdentifier
        }
    }

    private func save(_ manager: NETunnelProviderManager) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            manager.saveToPreferences { error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume() }
            }
        }
    }

    private func reload(_ manager: NETunnelProviderManager) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            manager.loadFromPreferences { error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume() }
            }
        }
    }

    private static func status(_ status: NEVPNStatus) -> String {
        switch status {
        case .connected: return "connected"
        case .connecting: return "connecting"
        case .reasserting: return "reconnecting"
        case .disconnecting: return "disconnecting"
        case .invalid, .disconnected: return "disconnected"
        @unknown default: return "unknown"
        }
    }
}
