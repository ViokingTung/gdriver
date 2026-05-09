import Foundation
import os.log

/// Manages communication with the `gdriver-daemon` process.
///
/// Uses the daemon's Unix-socket IPC (JSON-RPC 2.0 over NDJSON) for
/// querying sync state, toggling offline availability, and retrieving
/// share links.
actor DaemonConnection {

    private let log = OSLog(subsystem: "com.gdriver.findersync", category: "DaemonConnection")

    private var socketPath: String {
        if let tmp = ProcessInfo.processInfo.environment["TMPDIR"] {
            return "\(tmp)/gdriver.sock"
        }
        return "/tmp/gdriver.sock"
    }

    private var connection: FileHandle?
    private var requestId: UInt64 = 0
    private let maxRetries = 2

    // MARK: - Connection lifecycle

    func connect() throws {
        guard connection == nil else { return }

        let path = socketPath
        guard FileManager.default.fileExists(atPath: path) else {
            throw GDriverError.daemonNotRunning
        }

        let fd = open(path, O_RDWR)
        guard fd >= 0 else {
            throw GDriverError.connectionFailed(errno: errno)
        }

        connection = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        os_log(.debug, log: log, "connected to daemon at %{public}@", path)
    }

    func disconnect() {
        connection?.closeFile()
        connection = nil
    }

    // MARK: - RPC helpers

    private func nextRequestId() -> UInt64 {
        requestId += 1
        return requestId
    }

    private func sendRPC(method: String, params: [String: Any]) throws -> [String: Any] {
        let handle = try ensureConnection()

        let req: [String: Any] = [
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": nextRequestId(),
        ]

        let data = try JSONSerialization.data(withJSONObject: req)
        var line = String(data: data, encoding: .utf8) ?? ""
        line += "\n"

        guard let lineData = line.data(using: .utf8) else {
            throw GDriverError.serializationFailed
        }

        try handle.write(contentsOf: lineData)

        guard let responseLine = try readLine(from: handle) else {
            throw GDriverError.noResponse
        }

        guard let responseData = responseLine.data(using: .utf8),
              let response = try JSONSerialization.jsonObject(with: responseData) as? [String: Any]
        else {
            throw GDriverError.invalidResponse
        }

        if let error = response["error"] as? [String: Any] {
            let message = error["message"] as? String ?? "unknown error"
            throw GDriverError.rpcError(message)
        }

        guard let result = response["result"] as? [String: Any] else {
            throw GDriverError.invalidResponse
        }

        return result
    }

    private func ensureConnection() throws -> FileHandle {
        if connection == nil {
            try connect()
        }
        guard let conn = connection else {
            throw GDriverError.connectionFailed(errno: 0)
        }
        return conn
    }

    private func readLine(from handle: FileHandle) throws -> String? {
        var buffer = Data()
        let maxBytes = 65536

        for _ in 0..<maxBytes {
            let chunk = try handle.read(upToCount: 1)
            guard let byte = chunk, byte.count == 1 else {
                return buffer.isEmpty ? nil : String(data: buffer, encoding: .utf8)
            }

            if byte[0] == UInt8(ascii: "\n") {
                return String(data: buffer, encoding: .utf8)
            }
            buffer.append(byte)
        }

        return String(data: buffer, encoding: .utf8)
    }

    // MARK: - Finder Sync operations

    /// Query the sync state for a file at the given absolute path.
    func getSyncState(path: String) async throws -> String {
        let result = try sendRPC(method: "fs.get_sync_state", params: ["path": path])
        return result["state"] as? String ?? "cloud_only"
    }

    /// Mark a file as available offline or online-only.
    func setOffline(path: String, enabled: Bool) async throws {
        _ = try sendRPC(method: "fs.set_offline", params: [
            "path": path,
            "offline": enabled,
        ])
    }

    /// Retrieve the Google Drive share link for a file.
    func getShareLink(path: String) async throws -> String {
        let result = try sendRPC(method: "fs.get_share_link", params: ["path": path])
        guard let url = result["url"] as? String, !url.isEmpty else {
            throw GDriverError.notFound
        }
        return url
    }
}

// MARK: - Error types

enum GDriverError: LocalizedError {
    case daemonNotRunning
    case connectionFailed(errno: Int32)
    case serializationFailed
    case noResponse
    case invalidResponse
    case rpcError(String)
    case notFound

    var errorDescription: String? {
        switch self {
        case .daemonNotRunning:
            return "The gDriver daemon is not running."
        case .connectionFailed(let e):
            return "Failed to connect to daemon (errno: \(e))."
        case .serializationFailed:
            return "Failed to serialize RPC request."
        case .noResponse:
            return "No response from daemon."
        case .invalidResponse:
            return "Invalid response from daemon."
        case .rpcError(let msg):
            return "Daemon RPC error: \(msg)"
        case .notFound:
            return "File not found."
        }
    }
}
