// MockHTTPServer.swift — a tiny in-process HTTP/1.1 server for e2e tests.
//
// The aimux-ffi layer drives HTTP via Rust's `reqwest`, which does NOT go
// through `URLSession`. `URLProtocol`-based mocking therefore cannot
// intercept it. Instead we stand up a real local TCP socket that the FFI
// connects to, mirroring Node's `node:http` `createServer` pattern.
//
// The server listens on 127.0.0.1 with an OS-assigned port, accepts one
// request per connection, records the request body/path for assertions, and
// returns a pre-configured response (JSON or SSE). It uses `Connection:
// close` so each FFI call maps to exactly one connection.

import Foundation
#if canImport(Glibc)
import Glibc
#elseif canImport(Darwin)
import Darwin
#endif

/// A preset HTTP response returned by `MockHTTPServer`.
struct MockResponse {
    let status: Int
    let contentType: String
    let body: Data

    static func json(_ object: Any, status: Int = 200) -> MockResponse {
        let data = try! JSONSerialization.data(withJSONObject: object)
        return MockResponse(status: status, contentType: "application/json", body: data)
    }

    static func sse(_ body: String) -> MockResponse {
        return MockResponse(
            status: 200,
            contentType: "text/event-stream",
            body: body.data(using: .utf8) ?? Data()
        )
    }
}

/// Minimal POSIX-socket HTTP server for tests.
final class MockHTTPServer {

    /// The single response view (first of the queue). Kept for backward
    /// compatibility with callers that constructed the server with a single
    /// `response`. The queue (`responses`) is the source of truth at serve time.
    var response: MockResponse

    /// Responses returned in FIFO order, one per accepted request. Once
    /// exhausted, the last entry repeats so late/extra requests still get a
    /// well-defined answer instead of crashing.
    private var responses: [MockResponse]
    /// Number of requests served so far; indexes into `responses`.
    private var requestCount: Int = 0

    private var listenFd: Int32 = -1
    private let queue = DispatchQueue(label: "aimux.mock.http.server")
    private let lock = NSLock()
    private var stopped = false

    /// The most recently received request body (UTF-8).
    private(set) var lastRequestBody: String = ""
    /// The most recently received request path (e.g. "/chat/completions").
    private(set) var lastRequestPath: String = ""

    private var assignedPort: Int = 0
    /// Base URL the FFI should point at, e.g. `http://127.0.0.1:54321`.
    var baseURL: String { "http://127.0.0.1:\(assignedPort)" }

    /// Single-response constructor: every request returns the same response
    /// (backward compatible with existing tests).
    init(response: MockResponse) {
        self.responses = [response]
        self.response = response
    }

    /// Queue constructor: responses are returned in order, one per request.
    /// After the queue is exhausted the last response repeats. An empty queue
    /// falls back to a generic error response.
    init(responses: [MockResponse]) {
        if responses.isEmpty {
            self.responses = [.json(["error": "no responses"])]
        } else {
            self.responses = responses
        }
        self.response = self.responses[0]
    }

    /// Bind to 127.0.0.1 on an OS-assigned port and start accepting.
    func start() throws {
        listenFd = socket(AF_INET, Int32(SOCK_STREAM.rawValue), 0)
        guard listenFd >= 0 else {
            throw NSError(domain: "MockHTTPServer", code: 1, userInfo: [NSLocalizedDescriptionKey: "socket() failed"])
        }

        var yes: Int32 = 1
        _ = setsockopt(
            listenFd, SOL_SOCKET, SO_REUSEADDR,
            &yes, socklen_t(MemoryLayout<Int32>.size)
        )

        var addr = sockaddr_in()
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = 0 // 0 -> OS chooses a free port
        addr.sin_addr.s_addr = inet_addr("127.0.0.1")

        let bindRC = withUnsafePointer(to: &addr) { ptr -> Int32 in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                bind(listenFd, sa, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bindRC == 0 else {
            close(listenFd); listenFd = -1
            throw NSError(domain: "MockHTTPServer", code: 2, userInfo: [NSLocalizedDescriptionKey: "bind() failed"])
        }

        guard listen(listenFd, 16) == 0 else {
            close(listenFd); listenFd = -1
            throw NSError(domain: "MockHTTPServer", code: 3, userInfo: [NSLocalizedDescriptionKey: "listen() failed"])
        }

        // Retrieve the OS-assigned port.
        var actual = sockaddr_in()
        var len = socklen_t(MemoryLayout<sockaddr_in>.size)
        withUnsafeMutablePointer(to: &actual) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                _ = getsockname(listenFd, sa, &len)
            }
        }
        assignedPort = Int(UInt16(bigEndian: actual.sin_port))

        queue.async { [weak self] in self?.acceptLoop() }
    }

    /// Stop accepting and close the listening socket.
    func stop() {
        lock.lock()
        stopped = true
        let fd = listenFd
        listenFd = -1
        lock.unlock()
        if fd >= 0 { close(fd) }
    }

    deinit { stop() }

    // MARK: - internals

    private func acceptLoop() {
        while true {
            lock.lock()
            if stopped { lock.unlock(); return }
            let fd = listenFd
            lock.unlock()
            if fd < 0 { return }

            // Poll with a timeout so we can notice `stopped` promptly.
            var pfd = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
            let pr = poll(&pfd, 1, 200)
            if pr <= 0 { continue }
            if (pfd.revents & Int16(POLLIN)) == 0 { continue }

            let clientFd = accept(fd, nil, nil)
            if clientFd < 0 { continue }
            handle(clientFd)
            close(clientFd)
        }
    }

    /// Read one full HTTP request, record it, and send the preset response.
    private func handle(_ fd: Int32) {
        let (headerString, bodyData) = readRequest(fd)

        // Optional Expect: 100-continue handshake (reqwest/hyper may emit it).
        if headerString.lowercased().contains("expect: 100-continue") {
            _ = sendAll(fd, Data("HTTP/1.1 100 Continue\r\n\r\n".utf8))
        }

        // Record the request.
        let path = headerString
            .components(separatedBy: "\r\n")
            .first
            .map { $0.components(separatedBy: " ").dropFirst().first ?? "" } ?? ""
        let bodyStr = String(data: bodyData, encoding: .utf8) ?? ""

        lock.lock()
        lastRequestBody = bodyStr
        lastRequestPath = path
        lock.unlock()

        sendResponse(fd)
    }

    /// Read until headers complete (\r\n\r\n), then the Content-Length body.
    private func readRequest(_ fd: Int32) -> (String, Data) {
        let marker = Data([0x0D, 0x0A, 0x0D, 0x0A]) // \r\n\r\n
        var received = Data()
        let cap = 8192
        let buf = UnsafeMutablePointer<UInt8>.allocate(capacity: cap)
        defer { buf.deallocate() }

        var headerEnd: Int = -1
        // Read until the header terminator is found.
        while headerEnd < 0 {
            let n = recv(fd, UnsafeMutableRawPointer(buf), cap, 0)
            if n <= 0 { break }
            received.append(buf, count: n)
            if let r = received.range(of: marker) { headerEnd = r.upperBound }
        }
        if headerEnd < 0 {
            return ("", Data())
        }

        let headerData = received.subdata(in: 0..<headerEnd)
        let headerStr = String(data: headerData, encoding: .utf8) ?? ""

        // Determine Content-Length.
        var contentLength = 0
        for line in headerStr.components(separatedBy: "\r\n") {
            let lower = line.lowercased()
            if lower.hasPrefix("content-length:") {
                let value = line
                    .components(separatedBy: ":")
                    .dropFirst()
                    .joined()
                    .trimmingCharacters(in: .whitespaces)
                contentLength = Int(value) ?? 0
            }
        }

        // Read any remaining body bytes (small requests usually arrive whole).
        while (received.count - headerEnd) < contentLength {
            let n = recv(fd, UnsafeMutableRawPointer(buf), cap, 0)
            if n <= 0 { break }
            received.append(buf, count: n)
        }

        let bodyEnd = min(headerEnd + contentLength, received.count)
        let bodyData = received.subdata(in: headerEnd..<bodyEnd)
        return (headerStr, bodyData)
    }

    private func sendResponse(_ fd: Int32) {
        // Advance through the response queue, one entry per request. Once the
        // queue is exhausted the last entry repeats (safe: `responses` is never
        // empty by construction). The accept loop is serial, so this counter
        // needs no locking.
        let resp = requestCount < responses.count ? responses[requestCount] : responses.last!
        requestCount += 1

        let body = resp.body
        let statusText = (resp.status == 200) ? "OK" : "Error"

        // Build header bytes explicitly to control CRLF exactly.
        var packet = Data()
        packet.append("HTTP/1.1 \(resp.status) \(statusText)\r\n".data(using: .utf8)!)
        packet.append("Content-Type: \(resp.contentType)\r\n".data(using: .utf8)!)
        packet.append("Content-Length: \(body.count)\r\n".data(using: .utf8)!)
        packet.append("Connection: close\r\n".data(using: .utf8)!)
        packet.append("\r\n".data(using: .utf8)!)
        packet.append(body)
        _ = sendAll(fd, packet)
        // Signal EOF to the client's reader, then close.
        _ = shutdown(fd, Int32(SHUT_WR))
    }

    /// Write all bytes of `data` to `fd`.
    private func sendAll(_ fd: Int32, _ data: Data) -> Int {
        var sent = 0
        data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            var remaining = raw.count
            var ptr = raw.baseAddress
            while remaining > 0 {
                let n = write(fd, ptr, remaining)
                if n <= 0 { break }
                sent += n
                remaining -= n
                ptr = ptr?.advanced(by: n)
            }
        }
        return sent
    }
}
