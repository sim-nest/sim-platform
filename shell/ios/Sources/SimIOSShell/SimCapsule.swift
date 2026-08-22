import AVFAudio
import CSimPlatformIOS
import Foundation
import UserNotifications

public enum SimShellError: Error { case invalidABI, closed, capsule(String) }

/// Thin packet pump around the statically linked `sim_native_abi_v1` table.
/// UIKit/Foundation values are reduced to typed, bounded packets before calls.
@MainActor public final class SimCapsule {
    private let abi: UnsafePointer<SimNativeLibAbiV1>
    private var instance: UnsafeMutableRawPointer?
    private var securityScopedURLs: [String: URL] = [:]

    public init() throws {
        guard let table = sim_native_abi_v1(), table.pointee.abi_major == 1,
              table.pointee.struct_size >= MemoryLayout<SimNativeLibAbiV1>.size,
              let instantiate = table.pointee.instantiate,
              let instance = instantiate() else { throw SimShellError.invalidABI }
        abi = table
        self.instance = instance
    }

    deinit {
        for url in securityScopedURLs.values { url.stopAccessingSecurityScopedResource() }
        if let instance, let destroy = abi.pointee.destroy_instance { destroy(instance) }
    }

    @discardableResult public func lifecycle(_ state: String) throws -> Data {
        if state == "suspended" || state == "disconnected" { releaseAllDocumentGrants() }
        return try invoke("platform/lifecycle", ["type": "lifecycle", "state": state])
    }

    @discardableResult public func activate(action: String, url: URL? = nil) throws -> Data {
        var packet: [String: Any] = ["type": "activation", "action": action]
        if let url {
            let id = String(url.absoluteString.hashValue, radix: 16)
            guard url.startAccessingSecurityScopedResource() else {
                return try permission("shared-document", granted: false)
            }
            securityScopedURLs[id] = url
            packet["content"] = ["kind": "table", "mount": "ios-security-scoped", "key": [id]]
            _ = try invoke("platform/activation", ["type": "document-grant", "id": id, "active": true])
        }
        return try invoke("platform/activation", packet)
    }

    @discardableResult public func permission(_ name: String, granted: Bool) throws -> Data {
        try invoke("platform/activation", ["type": "permission", "permission": name, "granted": granted])
    }

    @discardableResult public func audioSessionAvailable(_ available: Bool) throws -> Data {
        if available { try? AVAudioSession.sharedInstance().setActive(true) }
        return try permission("microphone", granted: available)
    }

    @discardableResult public func notificationAuthorization(_ granted: Bool) throws -> Data {
        try permission("notifications", granted: granted)
    }

    @discardableResult public func backgroundExecution(_ allowed: Bool) throws -> Data {
        try invoke("platform/activation", ["type": "background-execution", "allowed": allowed])
    }

    @discardableResult public func memoryPressure() throws -> Data {
        try invoke("platform/activation", ["type": "memory-pressure"])
    }

    public func releaseAllDocumentGrants() {
        for (id, url) in securityScopedURLs {
            url.stopAccessingSecurityScopedResource()
            _ = try? invoke("platform/activation", ["type": "document-grant", "id": id, "active": false])
        }
        securityScopedURLs.removeAll()
    }

    private func invoke(_ function: String, _ object: [String: Any]) throws -> Data {
        guard let instance, let call = abi.pointee.call else { throw SimShellError.closed }
        let json = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        let encoded = json.withUnsafeBytes { raw -> SimCallResponse in
            let borrowed = SimBorrowedBytes(ptr: raw.bindMemory(to: UInt8.self).baseAddress, len: raw.count)
            return sim_ios_encode_input_json(borrowed)
        }
        let frame = try take(encoded)
        let response = frame.withUnsafeBytes { raw -> SimCallResponse in
            let borrowed = SimBorrowedBytes(ptr: raw.bindMemory(to: UInt8.self).baseAddress, len: raw.count)
            return function.withCString { call(instance, $0, borrowed) }
        }
        let outputFrame = try take(response)
        let decoded = outputFrame.withUnsafeBytes { raw -> SimCallResponse in
            sim_ios_decode_output_json(SimBorrowedBytes(ptr: raw.bindMemory(to: UInt8.self).baseAddress, len: raw.count))
        }
        return try take(decoded)
    }

    private func take(_ response: SimCallResponse) throws -> Data {
        if let error = response.error {
            let message = error.pointee.message.map(String.init(cString:)) ?? "capsule error"
            abi.pointee.destroy_error?(error)
            throw SimShellError.capsule(message)
        }
        let data = response.bytes.ptr.map { Data(bytes: $0, count: response.bytes.len) } ?? Data()
        abi.pointee.destroy_bytes?(response.bytes)
        return data
    }
}
