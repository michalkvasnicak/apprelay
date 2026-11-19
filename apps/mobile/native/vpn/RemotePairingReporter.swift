import Darwin
import Foundation
import os

final class RemotePairingReporter {
    private let logger = Logger(subsystem: "AppRelay", category: "RemotePairingReporter")
    private let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 15
        configuration.timeoutIntervalForResource = 15
        return URLSession(configuration: configuration, delegate: NoRedirects(), delegateQueue: nil)
    }()
    private var timer: DispatchSourceTimer?
    private var discovery: RemotePairingDiscovery?
    private var request: URLSessionDataTask?

    deinit { session.invalidateAndCancel() }

    func start(url: URL, appKey: String) {
        DispatchQueue.main.async { [self] in
            stopRefresh()
            let timer = DispatchSource.makeTimerSource(queue: .main)
            timer.schedule(deadline: .now(), repeating: .seconds(5), leeway: .seconds(1))
            timer.setEventHandler { [weak self] in
                self?.refresh(url: url, appKey: appKey)
            }
            self.timer = timer
            timer.resume()
        }
    }

    func stop() {
        DispatchQueue.main.async { [self] in stopRefresh() }
    }

    private func stopRefresh() {
        timer?.cancel()
        timer = nil
        discovery?.finish(nil)
        discovery = nil
        request?.cancel()
        request = nil
    }

    private func refresh(url: URL, appKey: String) {
        guard discovery == nil else { return }
        if let request, request.state != .completed { return }
        let probe = RemotePairingDiscovery { [weak self] port in
            guard let self else { return }
            discovery = nil
            guard let port else { return }
            report(port: port, url: url, appKey: appKey)
        }
        discovery = probe
        probe.start()
    }

    private func report(port: Int, url: URL, appKey: String) {
        let token: String
        do {
            guard let saved = try VpnKeychain.registrationToken() else {
                logger.error("Device token missing for RemotePairing report")
                return
            }
            token = saved
        } catch {
            logger.error("Device token unavailable for RemotePairing report")
            return
        }
        var report = URLRequest(url: url)
        report.httpMethod = "PUT"
        report.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        report.setValue(appKey, forHTTPHeaderField: "X-App-Key")
        report.setValue("application/json", forHTTPHeaderField: "Content-Type")
        report.httpBody = Data("{\"port\":\(port)}".utf8)
        let task = session.dataTask(with: report) { [weak self] _, response, error in
            if (error as? URLError)?.code == .cancelled { return }
            if let status = (response as? HTTPURLResponse)?.statusCode {
                if status == 204 {
                    self?.logger.info("Reported local RemotePairing port")
                } else {
                    self?.logger.error("RemotePairing port report rejected: HTTP \(status)")
                }
            } else if let error = error as? URLError {
                self?.logger.error("RemotePairing port report failed: URL error \(error.code.rawValue)")
            } else {
                self?.logger.error("RemotePairing port report failed without an HTTP response")
            }
        }
        request = task
        task.resume()
    }
}

private final class NoRedirects: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest,
                    completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

private final class RemotePairingDiscovery: NSObject, NetServiceBrowserDelegate, NetServiceDelegate {
    private let logger = Logger(subsystem: "AppRelay", category: "RemotePairingReporter")
    private let browser = NetServiceBrowser()
    private var services: [NetService] = []
    private var completion: ((Int?) -> Void)?

    init(completion: @escaping (Int?) -> Void) {
        self.completion = completion
    }

    func start() {
        browser.delegate = self
        browser.searchForServices(ofType: "_remotepairing._tcp.", inDomain: "local.")
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) { [weak self] in
            guard let self, completion != nil else { return }
            logger.error("No local RemotePairing service resolved before timeout")
            finish(nil)
        }
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool) {
        guard completion != nil else { return }
        services.append(service)
        service.delegate = self
        service.resolve(withTimeout: 4)
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didRemove service: NetService, moreComing: Bool) {
        services.removeAll { candidate in
            guard candidate.name == service.name,
                  candidate.type == service.type,
                  candidate.domain == service.domain else { return false }
            candidate.stop()
            return true
        }
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didNotSearch errorDict: [String: NSNumber]) {
        let code = errorDict[NetService.errorCode]?.intValue ?? 0
        logger.error("RemotePairing Bonjour browse failed: code \(code)")
        finish(nil)
    }

    func netServiceDidResolveAddress(_ sender: NetService) {
        guard services.contains(where: { $0 === sender }), (1...65535).contains(sender.port) else { return }
        let addresses = Self.localAddresses()
        let isLocal = sender.addresses?.contains { data in
            var address = sockaddr_storage()
            guard data.count >= MemoryLayout<sockaddr>.size,
                  data.count <= MemoryLayout<sockaddr_storage>.size else { return false }
            _ = withUnsafeMutableBytes(of: &address) { data.copyBytes(to: $0.bindMemory(to: UInt8.self)) }
            return withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { socketAddress in
                    guard Int(socketAddress.pointee.sa_len) <= data.count,
                          let host = Self.numericHost(socketAddress) else { return false }
                    return addresses.contains(host)
                }
            }
        }
        if isLocal == true { finish(sender.port) }
    }

    func finish(_ port: Int?) {
        guard let completion else { return }
        self.completion = nil
        browser.stop()
        for service in services { service.stop() }
        services.removeAll()
        completion(port)
    }

    private static func localAddresses() -> Set<String> {
        var interfaces: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&interfaces) == 0, let interfaces else { return [] }
        defer { freeifaddrs(interfaces) }
        var result: Set<String> = []
        var cursor: UnsafeMutablePointer<ifaddrs>? = interfaces
        while let entry = cursor {
            let interface = entry.pointee
            cursor = interface.ifa_next
            guard interface.ifa_flags & UInt32(IFF_UP) != 0,
                  interface.ifa_flags & UInt32(IFF_LOOPBACK) == 0,
                  let address = interface.ifa_addr, let host = numericHost(address) else { continue }
            result.insert(host)
        }
        return result
    }

    private static func numericHost(_ address: UnsafePointer<sockaddr>) -> String? {
        let family = Int32(address.pointee.sa_family)
        guard family == AF_INET || family == AF_INET6 else { return nil }
        var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
        guard getnameinfo(address, socklen_t(address.pointee.sa_len), &host, socklen_t(host.count),
                          nil, 0, NI_NUMERICHOST) == 0 else { return nil }
        return String(cString: host)
    }
}
