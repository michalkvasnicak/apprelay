import Darwin
import dnssd
import Foundation
import Network
import os

// Re-publish the server's unicast DNS-SD services on the local network.
final class BonjourProxy {
    private let queue = DispatchQueue(label: "AppRelay.BonjourProxy")
    private let logger = Logger(subsystem: "AppRelay", category: "BonjourProxy")
    private let allowArbitraryBonjour = Bundle.main.object(forInfoDictionaryKey: "AppRelayAllowArbitraryBonjour") as? Bool == true
    private let declaredServiceTypes = Set(
        (Bundle.main.object(forInfoDictionaryKey: "NSBonjourServices") as? [String] ?? [])
            .map { $0.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: ".")) }
    )
    private var receiver: DispatchSourceRead?
    private var services: [String: BonjourServiceProxy] = [:]
    private var draining: [ObjectIdentifier: BonjourServiceProxy] = [:]

    func start(address: String) {
        queue.async { [self] in
            logger.info("Starting announcement receiver: VPN UDP \(address, privacy: .public):5354")
            do {
                let fd = socket(AF_INET, SOCK_DGRAM, 0)
                guard fd >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
                var installed = false
                defer { if !installed { close(fd) } }
                var yes: Int32 = 1
                setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout.size(ofValue: yes)))
                setsockopt(fd, SOL_SOCKET, SO_REUSEPORT, &yes, socklen_t(MemoryLayout.size(ofValue: yes)))
                let interface = try bindToTunnel(fd, address: address)
                logger.info("Announcement socket bound to interface \(interface, privacy: .public), VPN IP \(address, privacy: .public)")
                var endpoint = ipv4SocketAddress(address, port: 5354)
                let result = withUnsafePointer(to: &endpoint) {
                    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                        Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
                    }
                }
                guard result == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
                _ = fcntl(fd, F_SETFL, O_NONBLOCK)
                let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
                source.setEventHandler { [weak self] in
                    guard let self else { return }
                    var bytes = [UInt8](repeating: 0, count: 65535)
                    var sender = sockaddr_in()
                    var senderLength = socklen_t(MemoryLayout<sockaddr_in>.size)
                    let count = withUnsafeMutablePointer(to: &sender) {
                        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                            recvfrom(fd, &bytes, bytes.count, 0, $0, &senderLength)
                        }
                    }
                    guard count >= 0 else {
                        let code = errno
                        if code != EAGAIN && code != EWOULDBLOCK {
                            logger.error("UDP receive failed on \(address, privacy: .public):5354, errno=\(code)")
                        }
                        return
                    }
                    var senderIP = sender.sin_addr
                    var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
                    let sourceIP = inet_ntop(AF_INET, &senderIP, &buffer, socklen_t(buffer.count))
                        .map { String(cString: $0) } ?? "unknown"
                    let sourcePort = UInt16(bigEndian: sender.sin_port)
                    logger.info("Received UDP announcement: \(count) bytes from \(sourceIP, privacy: .public):\(sourcePort) to \(address, privacy: .public):5354")
                    for announcement in ServiceAnnouncement.decode(Array(bytes.prefix(count))) {
                        guard allowArbitraryBonjour || declaredServiceTypes.contains(String(announcement.type.dropLast()).lowercased()) else {
                            logger.debug("Skipping Bonjour service type not declared in NSBonjourServices: \(announcement.type, privacy: .public)")
                            continue
                        }
                        let key = announcement.key
                        if announcement.ttl == 0 {
                            services[key]?.stop()
                            continue
                        }
                        guard announcement.address != address else { continue }
                        let proxy: BonjourServiceProxy
                        if let existing = services[key] {
                            proxy = existing
                        } else {
                            proxy = BonjourServiceProxy(queue: queue)
                            proxy.onWithdrawal = { [weak self, weak proxy] in
                                guard let self, let proxy, services[key] === proxy else { return }
                                services.removeValue(forKey: key)
                                if proxy.hasRelays { draining[ObjectIdentifier(proxy)] = proxy }
                            }
                            proxy.onDrained = { [weak self, weak proxy] in
                                guard let proxy else { return }
                                self?.draining.removeValue(forKey: ObjectIdentifier(proxy))
                            }
                            services[key] = proxy
                        }
                        proxy.received(announcement, address: address)
                    }
                }
                source.setCancelHandler { close(fd) }
                receiver = source
                installed = true
                source.resume()
                logger.info("Listening for Bonjour announcements on \(address, privacy: .public):5354 via \(interface, privacy: .public)")
            } catch {
                let detail = error as NSError
                logger.error("Cannot receive Bonjour announcements on \(address, privacy: .public):5354: \(error.localizedDescription, privacy: .public), domain=\(detail.domain, privacy: .public), code=\(detail.code)")
            }
        }
    }

    func stop() {
        queue.async { [self] in
            logger.info("Stopping Bonjour proxy and announcement receiver")
            receiver?.cancel()
            receiver = nil
            let active = Array(services.values) + Array(draining.values)
            services.removeAll()
            draining.removeAll()
            for proxy in active { proxy.stop(hard: true) }
        }
    }
}

// Each service owns its listener, registration, expiry, and active connections.
// All discovery state is confined to the receiver's serial queue.
private final class BonjourServiceProxy {
    private let queue: DispatchQueue
    private let logger = Logger(subsystem: "AppRelay", category: "BonjourProxy")
    private var listener: NWListener?
    private var advertisement: DNSServiceRef?
    private var relays: [UUID: ServiceRelay] = [:]
    private var expiry: DispatchWorkItem?
    private var current: ServiceAnnouncement?
    var onWithdrawal: (() -> Void)?
    var onDrained: (() -> Void)?
    var hasRelays: Bool { !relays.isEmpty }

    init(queue: DispatchQueue) { self.queue = queue }

    func stop(hard: Bool = false) {
        stopAdvertisement()
        if hard {
            for relay in relays.values { relay.stop() }
            relays.removeAll()
        }
        onWithdrawal?()
    }

    private func publish(_ announcement: ServiceAnnouncement, port: UInt16) throws {
        // NetService publishes on every interface. iOS was resolving our own
        // service on en2, then stalling before the local TCP handshake. Scope
        // discovery to iPhone Wi-Fi (en0), not USB/auxiliary interfaces or utun.
        let interface = if_nametoindex("en0")
        guard interface != 0 else { throw POSIXError(.ENXIO) }
        var service: DNSServiceRef?
        let result = announcement.txt.withUnsafeBytes { txt in
            DNSServiceRegister(
                &service, DNSServiceFlags(kDNSServiceFlagsNoAutoRename), interface,
                announcement.name, announcement.type, announcement.domain, nil,
                port.bigEndian, UInt16(txt.count), txt.baseAddress,
                { service, _, error, name, type, domain, context in
                    guard let context else { return }
                    let proxy = Unmanaged<BonjourServiceProxy>.fromOpaque(context).takeUnretainedValue()
                    guard proxy.advertisement == service else { return }
                    guard error == kDNSServiceErr_NoError else {
                        proxy.logger.error("Bonjour publication failed: DNS-SD error=\(error)")
                        proxy.stop()
                        return
                    }
                    let publishedName = name.map { String(cString: $0) } ?? "unknown"
                    let publishedType = type.map { String(cString: $0) } ?? "unknown"
                    let publishedDomain = domain.map { String(cString: $0) } ?? "unknown"
                    proxy.logger.info("Bonjour proxy advertisement published: name=\(publishedName, privacy: .public), type=\(publishedType, privacy: .public), domain=\(publishedDomain, privacy: .public), interface=en0, local port=\(proxy.listener?.port?.rawValue ?? 0)")
                }, Unmanaged.passUnretained(self).toOpaque()
            )
        }
        guard result == kDNSServiceErr_NoError, let service else {
            if let service { DNSServiceRefDeallocate(service) }
            throw NSError(domain: "AppRelay.BonjourProxy.DNSService", code: Int(result))
        }
        let scheduling = DNSServiceSetDispatchQueue(service, queue)
        guard scheduling == kDNSServiceErr_NoError else {
            DNSServiceRefDeallocate(service)
            throw NSError(domain: "AppRelay.BonjourProxy.DNSService", code: Int(scheduling))
        }
        advertisement = service
    }

    private func stopAdvertisement() {
        if listener != nil || advertisement != nil {
            logger.info("Withdrawing Bonjour advertisement; closing listener and draining \(self.relays.count) relays")
        }
        expiry?.cancel()
        expiry = nil
        listener?.cancel()
        listener = nil
        current = nil
        // A goodbye may overtake the final TCP response. Existing
        // relays must deliver that response and EOF before being removed.
        for relay in relays.values { relay.drain() }
        if let service = advertisement {
            advertisement = nil
            DNSServiceRefDeallocate(service)
        }
    }

    func received(_ announcement: ServiceAnnouncement, address: String) {
        expiry?.cancel()
        if current?.endpointAndMetadata != announcement.endpointAndMetadata {
            logger.info("Received complete Bonjour announcement; starting local proxy")
            stopAdvertisement()
            do {
                let parameters = announcement.isUDP ? NWParameters.udp : NWParameters.tcp
                parameters.includePeerToPeer = false
                // Do not require en0 on the listener: connections from this
                // iPhone to its own Wi-Fi address can arrive through loopback.
                let local = try NWListener(using: parameters)
                local.newConnectionHandler = { [weak self, weak local] connection in
                    guard let self, listener === local else { connection.cancel(); return }
                    logger.info("Accepted Bonjour connection from \(String(describing: connection.endpoint), privacy: .public); forwarding to \(announcement.address, privacy: .public):\(announcement.port)")
                    let id = UUID()
                    let relay: ServiceRelay = announcement.isUDP
                        ? DatagramRelay(connection: connection, destination: announcement.address,
                                        port: announcement.port, tunnelAddress: address)
                        : TCPRelay(connection: connection, destination: announcement.address,
                                   port: announcement.port, tunnelAddress: address)
                    relays[id] = relay
                    // Keep the owner alive while withdrawn services drain.
                    relay.start { [self] in
                        queue.async { [self] in
                            relays.removeValue(forKey: id)
                            if relays.isEmpty { onDrained?() }
                        }
                    }
                }
                local.stateUpdateHandler = { [weak self, weak local] state in
                    guard let self, listener === local else { return }
                    logger.info("Bonjour listener state: \(String(describing: state), privacy: .public)")
                    switch state {
                    case .ready:
                        guard let port = local?.port, advertisement == nil else { return }
                        do {
                            try publish(announcement, port: port.rawValue)
                            logger.info("Publishing Bonjour proxy on en0: local port=\(port.rawValue), upstream=\(announcement.address, privacy: .public):\(announcement.port), VPN IP=\(address, privacy: .public)")
                        } catch {
                            logger.error("Cannot publish Bonjour proxy: \(String(describing: error), privacy: .public)")
                            stop()
                        }
                    case .failed(let error):
                        logger.error("Bonjour listener failed: \(error.localizedDescription, privacy: .public)")
                        stop()
                    default: break
                    }
                }
                listener = local
                current = announcement
                local.start(queue: queue)
            } catch {
                logger.error("Cannot start Bonjour proxy: \(error.localizedDescription, privacy: .public)")
                stop()
                return
            }
        }
        let timeout = DispatchWorkItem { [weak self] in
            self?.stop()
        }
        expiry = timeout
        queue.asyncAfter(deadline: .now() + Double(announcement.ttl), execute: timeout)
    }
}

// Bind server-side sockets explicitly to utun, rather than letting provider
// traffic bypass its own VPN. No service payload or TXT data is logged.
@discardableResult
private func bindToTunnel(_ fd: Int32, address: String) throws -> String {
    var interfaces: UnsafeMutablePointer<ifaddrs>?
    guard getifaddrs(&interfaces) == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .ENXIO) }
    defer { freeifaddrs(interfaces) }
    var cursor = interfaces
    while let entry = cursor {
        defer { cursor = entry.pointee.ifa_next }
        guard let value = entry.pointee.ifa_addr, value.pointee.sa_family == AF_INET else { continue }
        let local = value.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { $0.pointee.sin_addr }
        var target = in_addr()
        guard inet_pton(AF_INET, address, &target) == 1, local.s_addr == target.s_addr else { continue }
        var index = if_nametoindex(entry.pointee.ifa_name)
        guard setsockopt(fd, IPPROTO_IP, IP_BOUND_IF, &index, socklen_t(MemoryLayout.size(ofValue: index))) == 0 else {
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .ENXIO)
        }
        return String(cString: entry.pointee.ifa_name)
    }
    throw POSIXError(.ENXIO)
}

private func ipv4SocketAddress(_ address: String, port: UInt16) -> sockaddr_in {
    var value = sockaddr_in()
    value.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
    value.sin_family = sa_family_t(AF_INET)
    value.sin_port = port.bigEndian
    inet_pton(AF_INET, address, &value.sin_addr)
    return value
}

private protocol ServiceRelay: AnyObject {
    func start(completion: @escaping () -> Void)
    func stop()
    func drain()
}

private final class TCPRelay: ServiceRelay {
    private let connection: NWConnection
    private let destination: String
    private let port: UInt16
    private let tunnelAddress: String
    private let queue = DispatchQueue(label: "AppRelay.TCPRelay")
    private let logger = Logger(subsystem: "AppRelay", category: "BonjourProxy")
    private var channel: DispatchIO?
    private var finished = false
    private var drainTimeout: DispatchWorkItem?
    private var clientReadClosed = false
    private var serverReadClosed = false
    private var pendingClientSends = 0
    private var sendingClientEOF = false
    private var clientEOFSent = false
    private var bytesToServer = 0
    private var bytesToClient = 0
    private var completion: (() -> Void)?

    init(connection: NWConnection, destination: String, port: UInt16, tunnelAddress: String) {
        self.connection = connection
        self.destination = destination
        self.port = port
        self.tunnelAddress = tunnelAddress
    }

    func start(completion: @escaping () -> Void) {
        queue.async { [self] in
            self.completion = completion
            let fd = socket(AF_INET, SOCK_STREAM, 0)
            guard fd >= 0 else {
                logger.error("Cannot create Bonjour TCP socket: errno=\(errno)")
                finish()
                return
            }
            do {
                let interface = try bindToTunnel(fd, address: tunnelAddress)
                logger.info("Connecting relay: client=\(String(describing: self.connection.endpoint), privacy: .public), VPN=\(self.tunnelAddress, privacy: .public), interface=\(interface, privacy: .public), server=\(self.destination, privacy: .public):\(self.port)")
            } catch {
                logger.error("Cannot bind Bonjour relay to VPN IP \(self.tunnelAddress, privacy: .public): \(error.localizedDescription, privacy: .public)")
                close(fd)
                finish()
                return
            }
            var yes: Int32 = 1
            setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &yes, socklen_t(MemoryLayout.size(ofValue: yes)))
            var timeout: UInt32 = 5
            setsockopt(fd, IPPROTO_TCP, TCP_CONNECTIONTIMEOUT, &timeout, socklen_t(MemoryLayout.size(ofValue: timeout)))
            var endpoint = ipv4SocketAddress(destination, port: port)
            let result = withUnsafePointer(to: &endpoint) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }
            guard result == 0 else {
                logger.error("Cannot connect Bonjour relay to \(self.destination, privacy: .public):\(self.port) via VPN \(self.tunnelAddress, privacy: .public): errno=\(errno)")
                close(fd)
                finish()
                return
            }
            logger.info("Bonjour relay connected to \(self.destination, privacy: .public):\(self.port) through VPN \(self.tunnelAddress, privacy: .public)")
            channel = DispatchIO(type: .stream, fileDescriptor: fd, queue: queue) { _ in close(fd) }
            channel?.setLimit(lowWater: 1)
            channel?.setLimit(highWater: 65536)
            connection.stateUpdateHandler = { [weak self] state in
                guard let self else { return }
                switch state {
                case .ready:
                    logger.info("Local Bonjour connection ready: endpoint=\(String(describing: self.connection.endpoint), privacy: .public), path=\(String(describing: self.connection.currentPath), privacy: .public)")
                    readLocal()
                    readRemote()
                case .failed(let error):
                    logger.error("Local Bonjour connection failed: \(error.localizedDescription, privacy: .public)")
                    finish()
                case .cancelled: finish()
                default: break
                }
            }
            connection.start(queue: queue)
        }
    }

    func stop() { queue.async { [self] in finish() } }

    func drain() { queue.async { [self] in beginDraining() } }

    private func beginDraining() {
        guard !finished, drainTimeout == nil else { return }
        // Bound cleanup if either endpoint never completes its half-close.
        // Normal completion closes immediately, without waiting for this timer.
        let timeout = DispatchWorkItem { [weak self] in
            guard let self else { return }
            logger.error("Bonjour relay drain timed out")
            finish()
        }
        drainTimeout = timeout
        queue.asyncAfter(deadline: .now() + 10, execute: timeout)
    }

    private func finish() {
        guard !finished else { return }
        finished = true
        drainTimeout?.cancel()
        drainTimeout = nil
        logger.info("Closing relay to \(self.destination, privacy: .public):\(self.port): client-to-server=\(self.bytesToServer) bytes, server-to-client=\(self.bytesToClient) bytes")
        connection.cancel()
        channel?.close(flags: .stop)
        channel = nil
        completion?()
        completion = nil
    }

    private func readLocal() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, complete, error in
            guard let self, !finished, let channel else { return }
            if let error {
                logger.error("Bonjour client read failed: \(error.localizedDescription, privacy: .public)")
                finish()
                return
            }
            guard let data, !data.isEmpty else {
                if complete { closeClientRead() }
                else { readLocal() }
                return
            }
            let bytes = data.withUnsafeBytes { DispatchData(bytes: $0) }
            channel.write(offset: 0, data: bytes, queue: queue) { [weak self] done, _, writeError in
                guard let self, !finished, done else { return }
                if writeError != 0 {
                    logger.error("Bonjour server write failed: errno=\(writeError)")
                    finish()
                } else {
                    bytesToServer += data.count
                    if complete { closeClientRead() }
                    else { readLocal() }
                }
            }
        }
    }

    private func closeClientRead() {
        clientReadClosed = true
        logger.info("Bonjour client closed its stream; draining server response")
        if let channel { Darwin.shutdown(channel.fileDescriptor, SHUT_WR) }
        beginDraining()
        if clientEOFSent { finish() }
    }

    private func readRemote() {
        // One streaming read, delivered in bounded chunks until EOF. DispatchIO
        // can report EOF before NWConnection finishes processing earlier sends.
        channel?.read(offset: 0, length: Int.max, queue: queue) { [weak self] done, bytes, error in
            guard let self, !finished else { return }
            guard error == 0 else {
                logger.error("Bonjour server read failed: errno=\(error)")
                finish()
                return
            }
            if let bytes, !bytes.isEmpty {
                pendingClientSends += 1
                connection.send(content: Data(bytes), completion: .contentProcessed { [weak self] error in
                    guard let self, !finished else { return }
                    pendingClientSends -= 1
                    if let error {
                        logger.error("Bonjour client write failed: \(error.localizedDescription, privacy: .public)")
                        finish()
                    } else {
                        bytesToClient += bytes.count
                        sendClientEOFIfDrained()
                    }
                })
            }
            if done {
                serverReadClosed = true
                logger.info("Bonjour server closed its stream; draining \(self.pendingClientSends) pending sends")
                beginDraining()
                sendClientEOFIfDrained()
            }
        }
    }

    private func sendClientEOFIfDrained() {
        guard !finished, serverReadClosed, pendingClientSends == 0, !sendingClientEOF else { return }
        sendingClientEOF = true
        connection.send(content: nil, contentContext: .finalMessage, isComplete: true,
                        completion: .contentProcessed { [weak self] error in
            guard let self, !finished else { return }
            if let error {
                logger.error("Bonjour client EOF failed: \(error.localizedDescription, privacy: .public)")
                finish()
                return
            }
            clientEOFSent = true
            logger.info("Final Bonjour response forwarded; sent EOF to client")
            if clientReadClosed { finish() }
        })
    }
}

// One connected tunnel socket per local UDP peer preserves datagram boundaries
// and routes replies back to that peer, rather than treating UDP as a stream.
private final class DatagramRelay: ServiceRelay {
    private let connection: NWConnection
    private let destination: String
    private let port: UInt16
    private let tunnelAddress: String
    private let queue = DispatchQueue(label: "AppRelay.DatagramRelay")
    private let logger = Logger(subsystem: "AppRelay", category: "BonjourProxy")
    private var receiver: DispatchSourceRead?
    private var socketFD: Int32 = -1
    private var timeout: DispatchWorkItem?
    private var draining = false
    private var finished = false
    private var completion: (() -> Void)?

    init(connection: NWConnection, destination: String, port: UInt16, tunnelAddress: String) {
        self.connection = connection
        self.destination = destination
        self.port = port
        self.tunnelAddress = tunnelAddress
    }

    func start(completion: @escaping () -> Void) {
        queue.async { [self] in
            self.completion = completion
            let fd = socket(AF_INET, SOCK_DGRAM, 0)
            guard fd >= 0 else { finish(); return }
            do {
                try bindToTunnel(fd, address: tunnelAddress)
                var endpoint = ipv4SocketAddress(destination, port: port)
                let result = withUnsafePointer(to: &endpoint) {
                    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                        Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
                    }
                }
                guard result == 0, fcntl(fd, F_SETFL, O_NONBLOCK) == 0 else {
                    throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
                }
            } catch {
                logger.error("Cannot open VPN UDP relay: \(error.localizedDescription, privacy: .public)")
                close(fd)
                finish()
                return
            }
            socketFD = fd
            let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
            source.setEventHandler { [weak self] in self?.readRemote() }
            source.setCancelHandler { close(fd) }
            receiver = source
            source.resume()
            connection.stateUpdateHandler = { [weak self] state in
                guard let self, !finished else { return }
                switch state {
                case .ready: readLocal()
                case .failed, .cancelled: finish()
                default: break
                }
            }
            connection.start(queue: queue)
            refreshTimeout()
        }
    }

    func stop() { queue.async { [self] in finish() } }

    func drain() {
        queue.async { [self] in
            guard !finished, !draining else { return }
            draining = true
            scheduleTimeout(seconds: 10)
        }
    }

    private func refreshTimeout() {
        if !draining { scheduleTimeout(seconds: 60) }
    }

    private func scheduleTimeout(seconds: Double) {
        timeout?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.finish() }
        timeout = work
        queue.asyncAfter(deadline: .now() + seconds, execute: work)
    }

    private func readLocal() {
        connection.receiveMessage { [weak self] data, _, _, error in
            guard let self, !finished else { return }
            guard error == nil else { finish(); return }
            if let data {
                let sent = data.withUnsafeBytes { bytes in
                    Darwin.send(socketFD, bytes.baseAddress, bytes.count, 0)
                }
                guard sent == data.count else { finish(); return }
                refreshTimeout()
            }
            readLocal()
        }
    }

    private func readRemote() {
        guard !finished else { return }
        var bytes = [UInt8](repeating: 0, count: 65535)
        let count = recv(socketFD, &bytes, bytes.count, 0)
        guard count >= 0 else {
            if errno != EAGAIN && errno != EWOULDBLOCK { finish() }
            return
        }
        refreshTimeout()
        connection.send(content: Data(bytes.prefix(count)), completion: .contentProcessed { [weak self] error in
            if error != nil { self?.finish() }
        })
    }

    private func finish() {
        guard !finished else { return }
        finished = true
        timeout?.cancel()
        timeout = nil
        connection.cancel()
        receiver?.cancel()
        receiver = nil
        socketFD = -1
        completion?()
        completion = nil
    }
}

// Decode all service instances in a complete server announcement. DNS names
// remain label arrays so dots in instance names and DNS compression are safe.
// A PTR-only or SRV-only goodbye can withdraw an existing service.
private struct ServiceAnnouncement {
    let name: String
    let type: String
    let domain: String
    let address: String
    let port: UInt16
    let txt: Data
    let ttl: UInt32

    var key: String { [name, type, domain].map { $0.lowercased() }.joined(separator: "\u{0}") }
    var isUDP: Bool { type.lowercased().hasSuffix("._udp.") }

    // TTL refreshes must not restart a working listener.
    var endpointAndMetadata: Metadata { Metadata(address: address, port: port, txt: txt) }
    struct Metadata: Equatable {
        let address: String
        let port: UInt16
        let txt: Data
    }

    static func decode(_ bytes: [UInt8]) -> [ServiceAnnouncement] {
        func number(_ offset: Int) -> Int { Int(bytes[offset]) << 8 | Int(bytes[offset + 1]) }
        func dnsName(_ offset: inout Int) -> [String]? {
            var labels: [String] = []
            var cursor = offset
            var jumped = false
            for _ in 0..<128 {
                guard cursor < bytes.count else { return nil }
                let length = Int(bytes[cursor])
                if length == 0 {
                    if !jumped { offset = cursor + 1 }
                    return labels
                }
                if length & 0xc0 == 0xc0 {
                    guard cursor + 1 < bytes.count else { return nil }
                    if !jumped { offset = cursor + 2 }
                    cursor = (length & 0x3f) << 8 | Int(bytes[cursor + 1])
                    jumped = true
                } else {
                    guard length <= 63, cursor + 1 + length <= bytes.count,
                          let label = String(bytes: bytes[(cursor + 1)..<(cursor + 1 + length)], encoding: .utf8),
                          !label.contains("\u{0}") else { return nil }
                    labels.append(label)
                    cursor += 1 + length
                    if !jumped { offset = cursor }
                }
            }
            return nil
        }
        func normalized(_ name: [String]) -> [String] { name.map { $0.lowercased() } }
        func escaped(_ label: String) -> String {
            label.replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: ".", with: "\\.")
        }
        guard bytes.count >= 12, bytes[2] & 0x80 != 0 else { return [] }
        var offset = 12
        for _ in 0..<number(4) {
            guard dnsName(&offset) != nil, offset + 4 <= bytes.count else { return [] }
            offset += 4
        }
        var instances: [[String]: (labels: [String], ttl: UInt32)] = [:]
        var servers: [[String]: (host: [String], port: UInt16, ttl: UInt32)] = [:]
        var texts: [[String]: (data: Data, ttl: UInt32)] = [:]
        var addresses: [[String]: (address: String, ttl: UInt32)] = [:]
        for _ in 0..<(number(6) + number(8) + number(10)) {
            guard let labels = dnsName(&offset), offset + 10 <= bytes.count else { return [] }
            let owner = normalized(labels)
            let type = number(offset)
            let recordClass = number(offset + 2) & 0x7fff
            let ttl = UInt32(number(offset + 4)) << 16 | UInt32(number(offset + 6))
            let length = number(offset + 8)
            offset += 10
            let end = offset + length
            guard end <= bytes.count else { return [] }
            var dataOffset = offset
            if recordClass == 1 {
                switch type {
                case 12:
                    if let instance = dnsName(&dataOffset), dataOffset <= end {
                        let key = normalized(instance)
                        instances[key] = (instance, min(instances[key]?.ttl ?? ttl, ttl))
                    }
                case 33 where length >= 7:
                    let port = UInt16(number(offset + 4))
                    dataOffset += 6
                    if let host = dnsName(&dataOffset), dataOffset <= end {
                        servers[owner] = (normalized(host), port, ttl)
                        if instances[owner] == nil { instances[owner] = (labels, ttl) }
                    }
                case 16: texts[owner] = (Data(bytes[offset..<end]), ttl)
                case 1 where length == 4:
                    addresses[owner] = (bytes[offset..<end].map(String.init).joined(separator: "."), ttl)
                default: break
                }
            }
            offset = end
        }
        var announcements: [ServiceAnnouncement] = []
        for (key, instance) in instances {
            let labels = instance.labels
            guard labels.count >= 4, labels[1].hasPrefix("_"),
                  ["_tcp", "_udp"].contains(labels[2].lowercased()) else { continue }
            let type = labels[1...2].map(escaped).joined(separator: ".") + "."
            let domain = labels.dropFirst(3).map(escaped).joined(separator: ".") + "."
            let server = servers[key]
            let text = texts[key]
            let host = server.flatMap { addresses[$0.host] }
            guard let ttl = [instance.ttl, server?.ttl, text?.ttl, host?.ttl].compactMap({ $0 }).min() else { continue }
            guard ttl == 0 || (server != nil && text != nil && host != nil) else { continue }
            announcements.append(ServiceAnnouncement(
                name: labels[0], type: type, domain: domain,
                address: host?.address ?? "", port: server?.port ?? 0,
                txt: text?.data ?? Data(), ttl: ttl
            ))
        }
        return announcements
    }
}
