import Foundation
import Network
import NetworkExtension
import React

@objc(AppRelayVPN)
final class AppRelayVPN: NSObject {
    private var operationInProgress = false
    private var providerIdentifier: String { Bundle.main.bundleIdentifier! + ".PacketTunnel" }
    // Monitor Wi-Fi specifically so a VPN route cannot mask the underlying interface.
    private let wifiMonitor = NWPathMonitor(requiredInterfaceType: .wifi)
    private var wifiConnected: Bool?

    override init() {
        super.init()
        wifiMonitor.pathUpdateHandler = { [weak self] path in
            self?.wifiConnected = path.status == .satisfied
        }
        wifiMonitor.start(queue: .main)
    }

    deinit { wifiMonitor.cancel() }

    @objc static func requiresMainQueueSetup() -> Bool { true }

    @objc(getWifiConnected:rejecter:)
    func getWifiConnected(_ resolve: @escaping RCTPromiseResolveBlock,
                          rejecter _: @escaping RCTPromiseRejectBlock) {
        Task { @MainActor in
            if let wifiConnected { resolve(wifiConnected) }
            else { resolve(NSNull()) }
        }
    }

    @MainActor private func requireWifi() throws {
        guard wifiConnected == true else { throw VpnError.wifiRequired }
    }

    @objc(getIdentity:rejecter:)
    func getIdentity(_ resolve: @escaping RCTPromiseResolveBlock, rejecter reject: @escaping RCTPromiseRejectBlock) {
        Task { @MainActor in
            do {
                guard !operationInProgress else { throw VpnError.busy }
                if let identity = try VpnKeychain.identity() {
                    resolve(["publicKey": identity.publicKey])
                    return
                }
                // A missing or corrupt key invalidates any saved token and tunnel profile.
                let identity = try await replaceIdentity(discardPreviousStorage: true)
                resolve(["publicKey": identity.publicKey])
            } catch { reject("VPN_IDENTITY", error.localizedDescription, error) }
        }
    }

    @objc(getRegistrationToken:rejecter:)
    func getRegistrationToken(_ resolve: @escaping RCTPromiseResolveBlock,
                              rejecter reject: @escaping RCTPromiseRejectBlock) {
        Task { @MainActor in
            do {
                guard !operationInProgress else { throw VpnError.busy }
                resolve(try VpnKeychain.registrationToken())
            } catch { reject("VPN_CREDENTIAL", error.localizedDescription, error) }
        }
    }

    @objc(saveRegistrationToken:publicKey:resolver:rejecter:)
    func saveRegistrationToken(_ token: String, publicKey: String,
                               resolver resolve: @escaping RCTPromiseResolveBlock,
                               rejecter reject: @escaping RCTPromiseRejectBlock) {
        Task { @MainActor in
            do {
                guard !operationInProgress else { throw VpnError.busy }
                guard try VpnKeychain.identity()?.publicKey == publicKey else { throw VpnError.missingKey }
                try VpnKeychain.saveRegistrationToken(token)
                resolve(nil)
            } catch { reject("VPN_CREDENTIAL", error.localizedDescription, error) }
        }
    }

    @objc(resetRegistration:rejecter:)
    func resetRegistration(_ resolve: @escaping RCTPromiseResolveBlock,
                           rejecter reject: @escaping RCTPromiseRejectBlock) {
        Task { @MainActor in
            do {
                let identity = try await replaceIdentity()
                resolve(["publicKey": identity.publicKey])
            } catch { reject("VPN_RESET", error.localizedDescription, error) }
        }
    }

    @MainActor private func replaceIdentity(discardPreviousStorage: Bool = false) async throws -> VpnKeychain.Identity {
        guard !operationInProgress else { throw VpnError.busy }
        operationInProgress = true
        defer { operationInProgress = false }

        for manager in try await loadManagers() {
            let connection = manager.connection
            connection.stopVPNTunnel()
            // Never delete an identity while a tunnel may still be using it.
            for _ in 0..<50 {
                if [.invalid, .disconnected].contains(connection.status) { break }
                try await Task.sleep(nanoseconds: 100_000_000)
            }
            guard [.invalid, .disconnected].contains(connection.status) else { throw VpnError.busy }
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                manager.removeFromPreferences { error in
                    if let error { continuation.resume(throwing: error) }
                    else { continuation.resume() }
                }
            }
        }

        try VpnKeychain.clearCredentials()
        if discardPreviousStorage { try VpnKeychain.discardPreviousStorage() }
        return try VpnKeychain.createIdentity()
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
                try requireWifi()
                let settings = try VpnSettings(values)
                guard let identity = try VpnKeychain.identity() else { throw VpnError.missingKey }
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
                try requireWifi()
                try manager.connection.startVPNTunnel()
                // This means startup was requested, not that the peer has handshaken.
                resolve(nil)
                #endif
            } catch { reject("VPN_CONNECT", error.localizedDescription, error) }
        }
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
                try requireWifi()
                manager?.connection.stopVPNTunnel()
                resolve(nil)
            } catch { reject("VPN_DISCONNECT", error.localizedDescription, error) }
        }
    }

    private func loadManager() async throws -> NETunnelProviderManager? {
        try await loadManagers().first
    }

    private func loadManagers() async throws -> [NETunnelProviderManager] {
        let managers: [NETunnelProviderManager] = try await withCheckedThrowingContinuation { continuation in
            NETunnelProviderManager.loadAllFromPreferences { managers, error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume(returning: managers ?? []) }
            }
        }
        return managers.filter {
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
