import Foundation

// Event wire format (matches Linux turm-core protocol):
// {"event": "terminal.output", "data": {"panel_id": "...", "text": "..."}}

// MARK: - EventBus

/// Broadcast hub for all turm events.
/// Subscribers hold an EventChannel that buffers events until the socket thread reads them.
final class EventBus: @unchecked Sendable {
    private let lock = NSLock()
    private var channels: [EventChannel] = []

    func subscribe() -> EventChannel {
        let ch = EventChannel()
        lock.withLock { channels.append(ch) }
        return ch
    }

    /// Broadcast an event to all live subscribers. Dead subscribers are pruned.
    func broadcast(event: String, data: [String: Any] = [:]) {
        let payload: [String: Any] = ["event": event, "data": data]
        guard
            let jsonData = try? JSONSerialization.data(withJSONObject: payload),
            let json = String(data: jsonData, encoding: .utf8)
        else { return }

        lock.withLock {
            channels.removeAll { !$0.send(json) }
        }
    }
}

// MARK: - EventChannel

/// Single-subscriber FIFO queue. The socket thread blocks on `receive()`
/// while the main thread pushes events via `send(_:)`.
final class EventChannel: @unchecked Sendable {
    private var queue: [String] = []
    private let sema = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var closed = false

    /// Returns false if the channel is already closed (subscriber disconnected).
    func send(_ event: String) -> Bool {
        lock.lock()
        guard !closed else { lock.unlock(); return false }
        queue.append(event)
        lock.unlock()
        sema.signal()
        return true
    }

    /// Blocks until an event is available. Returns nil when the channel is closed.
    func receive() -> String? {
        sema.wait()
        return lock.withLock {
            if closed, queue.isEmpty { return nil }
            return queue.isEmpty ? nil : queue.removeFirst()
        }
    }

    func close() {
        lock.withLock { closed = true }
        sema.signal()
    }
}

// MARK: - NSLock convenience

private extension NSLock {
    @discardableResult
    func withLock<T>(_ body: () -> T) -> T {
        lock(); defer { unlock() }
        return body()
    }
}
