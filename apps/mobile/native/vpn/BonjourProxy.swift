import Darwin
import dnssd
import Foundation
import Network
import os

// Bridge for the server's unicast DNS-SD pairing announcements.
final class BonjourProxy {
    private let queue = DispatchQueue(label: "AppRelay.BonjourProxy")
    private let logger = Logger(subsystem: "AppRelay", category: "BonjourProxy")
    private var receiver: DispatchSourceRead?
    private var listener: NWListener?
    private var advertisement: DNSServiceRef?
    private var relays: [UUID: PairingRelay] = [:]
    private var expiry: DispatchWorkItem?
    private var current: PairingAnnouncement?

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
                    guard let announcement = PairingAnnouncement(Array(bytes.prefix(count))) else {
                        logger.info("Ignoring UDP packet: no complete pairing PTR/SRV/TXT/A announcement")
                        return
                    }
                    received(announcement, address: address)
                }
                source.setCancelHandler { close(fd) }
                receiver = source
                installed = true
                source.resume()
                logger.info("Listening for pairing announcements on \(address, privacy: .public):5354 via \(interface, privacy: .public)")
            } catch {
                let detail = error as NSError
                logger.error("Cannot receive pairing announcements on \(address, privacy: .public):5354: \(error.localizedDescription, privacy: .public), domain=\(detail.domain, privacy: .public), code=\(detail.code)")
            }
        }
    }

    func stop() {
        queue.async { [self] in
            logger.info("Stopping Bonjour proxy and announcement receiver")
            receiver?.cancel()
            receiver = nil
            stopAdvertisement()
        }
    }

    private func publish(_ announcement: PairingAnnouncement, port: UInt16) throws {
        // NetService publishes on every interface. iOS was resolving our own
        // service on en2, then stalling before the local TCP handshake. Scope
        // discovery to iPhone Wi-Fi (en0), not USB/auxiliary interfaces or utun.
        let interface = if_nametoindex("en0")
        guard interface != 0 else { throw POSIXError(.ENXIO) }
        var service: DNSServiceRef?
        let result = announcement.txt.withUnsafeBytes { txt in
            DNSServiceRegister(
                &service, DNSServiceFlags(kDNSServiceFlagsNoAutoRename), interface,
                announcement.name, "_remotepairing-pairable-host._tcp.", "local.", nil,
                port.bigEndian, UInt16(txt.count), txt.baseAddress,
                { service, _, error, name, type, domain, context in
                    guard let context else { return }
                    let proxy = Unmanaged<BonjourProxy>.fromOpaque(context).takeUnretainedValue()
                    guard proxy.advertisement == service else { return }
                    guard error == kDNSServiceErr_NoError else {
                        proxy.logger.error("Bonjour publication failed: DNS-SD error=\(error)")
                        proxy.stopAdvertisement()
                        return
                    }
                    let publishedName = name.map { String(cString: $0) } ?? "unknown"
                    let publishedType = type.map { String(cString: $0) } ?? "unknown"
                    let publishedDomain = domain.map { String(cString: $0) } ?? "unknown"
                    proxy.logger.info("Pairing proxy Bonjour advertisement published: name=\(publishedName, privacy: .public), type=\(publishedType, privacy: .public), domain=\(publishedDomain, privacy: .public), interface=en0, local TCP port=\(proxy.listener?.port?.rawValue ?? 0)")
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
            logger.info("Withdrawing pairing advertisement; closing listener and \(self.relays.count) relays")
        }
        expiry?.cancel()
        expiry = nil
        listener?.cancel()
        listener = nil
        current = nil
        for relay in relays.values { relay.stop() }
        relays.removeAll()
        if let service = advertisement {
            advertisement = nil
            DNSServiceRefDeallocate(service)
        }
    }

    private func received(_ announcement: PairingAnnouncement, address: String) {
        logger.info("Decoded pairing announcement: server TCP=\(announcement.address, privacy: .public):\(announcement.port), TTL=\(announcement.ttl)s, TXT size=\(announcement.txt.count) bytes")
        guard announcement.address != address else {
            logger.info("Ignoring announcement pointing back to device VPN IP \(address, privacy: .public)")
            return
        }
        if announcement.ttl == 0 {
            logger.info("Received pairing service goodbye (TTL=0)")
            if current?.name == announcement.name { stopAdvertisement() }
            return
        }
        expiry?.cancel()
        if current != announcement {
            logger.info("Received complete pairing announcement; starting local proxy")
            stopAdvertisement()
            do {
                let parameters = NWParameters.tcp
                parameters.includePeerToPeer = false
                // Do not require en0 on the listener: connections from this
                // iPhone to its own Wi-Fi address can arrive through loopback.
                let local = try NWListener(using: parameters)
                local.newConnectionHandler = { [weak self, weak local] connection in
                    guard let self, listener === local else { connection.cancel(); return }
                    logger.info("Accepted pairing connection from \(String(describing: connection.endpoint), privacy: .public); forwarding to \(announcement.address, privacy: .public):\(announcement.port)")
                    let id = UUID()
                    let relay = PairingRelay(connection: connection, destination: announcement.address,
                                             port: announcement.port, tunnelAddress: address)
                    relays[id] = relay
                    relay.start { [weak self] in
                        self?.queue.async { [weak self] in self?.relays.removeValue(forKey: id) }
                    }
                }
                local.stateUpdateHandler = { [weak self, weak local] state in
                    guard let self, listener === local else { return }
                    logger.info("Pairing listener state: \(String(describing: state), privacy: .public)")
                    switch state {
                    case .ready:
                        guard let port = local?.port, advertisement == nil else { return }
                        do {
                            try publish(announcement, port: port.rawValue)
                            logger.info("Publishing pairing proxy on en0: local TCP port=\(port.rawValue), upstream=\(announcement.address, privacy: .public):\(announcement.port), VPN IP=\(address, privacy: .public)")
                        } catch {
                            logger.error("Cannot publish pairing proxy: \(String(describing: error), privacy: .public)")
                            stopAdvertisement()
                        }
                    case .failed(let error):
                        logger.error("Pairing listener failed: \(error.localizedDescription, privacy: .public)")
                        stopAdvertisement()
                    default: break
                    }
                }
                listener = local
                current = announcement
                local.start(queue: queue)
            } catch {
                logger.error("Cannot start pairing proxy: \(error.localizedDescription, privacy: .public)")
            }
        }
        let timeout = DispatchWorkItem { [weak self] in
            self?.logger.info("Pairing announcement TTL expired; withdrawing proxy")
            self?.stopAdvertisement()
        }
        expiry = timeout
        queue.asyncAfter(deadline: .now() + Double(announcement.ttl), execute: timeout)
    }
}

// Bind server-side sockets explicitly to utun, rather than letting provider
// traffic bypass its own VPN. No pairing payload or TXT data is logged.
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

private final class PairingRelay {
    private let connection: NWConnection
    private let destination: String
    private let port: UInt16
    private let tunnelAddress: String
    private let queue = DispatchQueue(label: "AppRelay.PairingRelay")
    private let logger = Logger(subsystem: "AppRelay", category: "BonjourProxy")
    private var channel: DispatchIO?
    private var finished = false
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
                logger.error("Cannot create pairing TCP socket: errno=\(errno)")
                finish()
                return
            }
            do {
                let interface = try bindToTunnel(fd, address: tunnelAddress)
                logger.info("Connecting relay: client=\(String(describing: self.connection.endpoint), privacy: .public), VPN=\(self.tunnelAddress, privacy: .public), interface=\(interface, privacy: .public), server=\(self.destination, privacy: .public):\(self.port)")
            } catch {
                logger.error("Cannot bind pairing relay to VPN IP \(self.tunnelAddress, privacy: .public): \(error.localizedDescription, privacy: .public)")
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
                logger.error("Cannot connect pairing relay to \(self.destination, privacy: .public):\(self.port) via VPN \(self.tunnelAddress, privacy: .public): errno=\(errno)")
                close(fd)
                finish()
                return
            }
            logger.info("Pairing relay connected to \(self.destination, privacy: .public):\(self.port) through VPN \(self.tunnelAddress, privacy: .public)")
            channel = DispatchIO(type: .stream, fileDescriptor: fd, queue: queue) { _ in close(fd) }
            channel?.setLimit(lowWater: 1)
            connection.stateUpdateHandler = { [weak self] state in
                guard let self else { return }
                switch state {
                case .ready:
                    logger.info("Local pairing connection ready: endpoint=\(String(describing: self.connection.endpoint), privacy: .public), path=\(String(describing: self.connection.currentPath), privacy: .public)")
                    readLocal()
                    readRemote()
                case .failed(let error):
                    logger.error("Local pairing connection failed: \(error.localizedDescription, privacy: .public)")
                    finish()
                case .cancelled: finish()
                default: break
                }
            }
            connection.start(queue: queue)
        }
    }

    func stop() { queue.async { [self] in finish() } }

    private func finish() {
        guard !finished else { return }
        finished = true
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
            guard let data, !data.isEmpty else {
                if let error { logger.error("Pairing client read failed: \(error.localizedDescription, privacy: .public)") }
                else { logger.info("Pairing client closed its stream") }
                finish()
                return
            }
            let bytes = data.withUnsafeBytes { DispatchData(bytes: $0) }
            channel.write(offset: 0, data: bytes, queue: queue) { [weak self] done, _, writeError in
                guard let self, done else { return }
                if writeError != 0 {
                    logger.error("Pairing server write failed: errno=\(writeError)")
                    finish()
                } else {
                    bytesToServer += data.count
                    if let error { logger.error("Pairing client read failed: \(error.localizedDescription, privacy: .public)") }
                    if error != nil || complete { finish() }
                    else { readLocal() }
                }
            }
        }
    }

    private func readRemote() {
        channel?.read(offset: 0, length: 65536, queue: queue) { [weak self] done, bytes, error in
            guard let self, !finished else { return }
            guard error == 0 else {
                logger.error("Pairing server read failed: errno=\(error)")
                finish()
                return
            }
            guard let bytes, !bytes.isEmpty else {
                if done { logger.info("Pairing server closed its stream"); finish() }
                return
            }
            connection.send(content: Data(bytes), completion: .contentProcessed { [weak self] error in
                guard let self else { return }
                if let error {
                    logger.error("Pairing client write failed: \(error.localizedDescription, privacy: .public)")
                    finish()
                } else {
                    bytesToClient += bytes.count
                    if done { readRemote() }
                }
            })
        }
    }
}

// Only decode the PTR/SRV/TXT/A records needed from a complete announcement.
// Supports DNS compression; does not rewrite or replay raw DNS packets.
private struct PairingAnnouncement: Equatable {
    let name: String
    let address: String
    let port: UInt16
    let txt: Data
    let ttl: UInt32

    init?(_ bytes: [UInt8]) {
        func number(_ offset: Int) -> Int { Int(bytes[offset]) << 8 | Int(bytes[offset + 1]) }
        func dnsName(_ offset: inout Int) -> String? {
            var labels: [String] = []
            var cursor = offset
            var jumped = false
            for _ in 0..<128 {
                guard cursor < bytes.count else { return nil }
                let length = Int(bytes[cursor])
                if length == 0 {
                    if !jumped { offset = cursor + 1 }
                    return labels.joined(separator: ".")
                }
                if length & 0xc0 == 0xc0 {
                    guard cursor + 1 < bytes.count else { return nil }
                    if !jumped { offset = cursor + 2 }
                    cursor = (length & 0x3f) << 8 | Int(bytes[cursor + 1])
                    jumped = true
                } else {
                    guard length <= 63, cursor + 1 + length <= bytes.count,
                          let label = String(bytes: bytes[(cursor + 1)..<(cursor + 1 + length)], encoding: .utf8) else { return nil }
                    labels.append(label)
                    cursor += 1 + length
                    if !jumped { offset = cursor }
                }
            }
            return nil
        }
        guard bytes.count >= 12, bytes[2] & 0x80 != 0 else { return nil }
        var offset = 12
        for _ in 0..<number(4) {
            guard dnsName(&offset) != nil, offset + 4 <= bytes.count else { return nil }
            offset += 4
        }
        var instances: [String] = []
        var servers: [String: (String, UInt16, UInt32)] = [:]
        var texts: [String: Data] = [:]
        var addresses: [String: String] = [:]
        for _ in 0..<(number(6) + number(8) + number(10)) {
            guard let owner = dnsName(&offset), offset + 10 <= bytes.count else { return nil }
            let type = number(offset)
            let ttl = UInt32(number(offset + 4)) << 16 | UInt32(number(offset + 6))
            let length = number(offset + 8)
            offset += 10
            let end = offset + length
            guard end <= bytes.count else { return nil }
            var dataOffset = offset
            switch type {
            case 12 where owner.lowercased() == "_remotepairing-pairable-host._tcp.local":
                if let instance = dnsName(&dataOffset), dataOffset <= end { instances.append(instance) }
            case 33 where length >= 7:
                let port = UInt16(number(offset + 4))
                dataOffset += 6
                if let host = dnsName(&dataOffset), dataOffset <= end { servers[owner] = (host, port, ttl) }
            case 16: texts[owner] = Data(bytes[offset..<end])
            case 1 where length == 4: addresses[owner] = bytes[offset..<end].map(String.init).joined(separator: ".")
            default: break
            }
            offset = end
        }
        for instance in instances {
            let suffix = "._remotepairing-pairable-host._tcp.local"
            guard instance.lowercased().hasSuffix(suffix), let (host, port, ttl) = servers[instance],
                  let address = addresses[host], let txt = texts[instance] else { continue }
            self.name = String(instance.dropLast(suffix.count))
            self.address = address
            self.port = port
            self.txt = txt
            self.ttl = ttl
            return
        }
        return nil
    }
}
