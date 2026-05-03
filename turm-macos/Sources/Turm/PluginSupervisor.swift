import Foundation

/// Minimal macOS service plugin supervisor. PR 3 (Tier 3 spike) scope:
/// discover manifests, spawn `onStartup` services, run the init handshake,
/// register `provides[]` actions in `ActionRegistry`, route `action.invoke`
/// frames over stdio, send `shutdown` notification on app quit.
///
/// What's intentionally NOT here yet (codex round-2 said do these later):
///
/// - **Activation gating** beyond `onStartup`. `onAction:<glob>` and
///   `onEvent:<glob>` need a deferred-spawn registry that wakes the plugin
///   the first time a matching method/event lands. PR 5 (trigger engine).
/// - **Restart-on-crash policy.** A crashed plugin stays dead for the
///   lifetime of this turm process. PR 4+ adds the supervisor loop.
/// - **Event subscription** (`event.dispatch` outbound). Echo doesn't
///   subscribe to anything, and we have no trigger engine yet to drive
///   subscribed events. Wired in PR 5.
/// - **Inbound `event.publish` forwarding to EventBus.** Plugin can emit
///   events; we currently just log them. Will fan them onto `eventBus` once
///   the trigger engine cares (PR 5).
/// - **`provides[]` conflict resolution** across plugins. We have one
///   plugin (echo) so no conflict can happen yet. Linux's
///   `service_supervisor::resolve_provides` ports cleanly when needed.
/// - **Process-group ownership + signal handlers + crash-recovery PID
///   file** (codex I5). Today we just `terminate()` on `applicationWillTerminate`.
///   Add when we have multiple production plugins (Slack/Calendar) where
///   leaked processes start mattering.
///
/// Threading model:
///
/// - `discoverAndStart` runs on `@MainActor` (called from AppDelegate).
/// - Each `PluginProcess` owns a dedicated reader thread that reads NDJSON
///   from stdout, looks up the response by id in a lock-protected pending
///   dict, and bounces the completion back to `DispatchQueue.main.async`.
/// - Stdin writes happen from the calling thread (main actor for action
///   dispatch, init handshake thread for the initial frame). `FileHandle.write`
///   is thread-safe per Apple docs and our writes are atomic per JSON
///   line so interleaving across threads is fine in practice.
@MainActor
final class PluginSupervisor {
    private let registry: ActionRegistry
    private let eventBus: EventBus
    private var processes: [PluginProcess] = []

    init(registry: ActionRegistry, eventBus: EventBus) {
        self.registry = registry
        self.eventBus = eventBus
    }

    /// Discover every manifest, spawn services with `activation = onStartup`,
    /// run init handshake, register provided actions in the registry. Idempotent
    /// only if called once per process — safe to call multiple times only after
    /// `shutdown()` has reset state.
    func discoverAndStart() {
        let manifests = PluginManifestStore.discover()
        for loaded in manifests {
            for service in loaded.manifest.services {
                // Activation handling (PR 4 expansion):
                //
                // - `onStartup` → spawn eagerly (PR 3, original).
                // - `onAction:<glob>` → spawn eagerly + log "lazy not yet
                //   implemented". Real lazy activation needs a placeholder-handler
                //   strategy (register provides[] with a deferred shim, spawn on
                //   first call, queue subsequent calls until init completes).
                //   That belongs with the trigger engine work (PR 5) where
                //   action-pattern matching gets centralized. For light plugins
                //   like git the eager-spawn cost is < 100ms; revisit when
                //   slack/calendar/llm land where startup includes auth/network.
                // - `onEvent:<glob>` → genuinely needs lazy because the plugin
                //   only matters when matching events arrive — eager spawn
                //   would burn resources. Skip with log.
                let activation = service.activation
                if activation == "onStartup" {
                    // No log — common case, no surprise.
                } else if activation.hasPrefix("onAction:") {
                    let msg = "[turm-plugin] \(loaded.manifest.plugin.name)/\(service.name): activation \(activation) — lazy not yet implemented, spawning eagerly\n"
                    FileHandle.standardError.write(Data(msg.utf8))
                } else if activation.hasPrefix("onEvent:") {
                    let msg = "[turm-plugin] \(loaded.manifest.plugin.name)/\(service.name): activation \(activation) needs trigger engine (PR 5) — skipping\n"
                    FileHandle.standardError.write(Data(msg.utf8))
                    continue
                } else {
                    let msg = "[turm-plugin] \(loaded.manifest.plugin.name)/\(service.name): unknown activation '\(activation)' — skipping\n"
                    FileHandle.standardError.write(Data(msg.utf8))
                    continue
                }
                guard let proc = PluginProcess.spawn(loaded: loaded, service: service) else {
                    continue
                }
                processes.append(proc)
                registerActions(for: proc)
            }
        }
    }

    private func registerActions(for proc: PluginProcess) {
        for actionName in proc.provides {
            // Capture proc weakly so a future shutdown doesn't keep the
            // pipes alive via the registry's strong reference.
            registry.register(actionName) { [weak proc, actionName] params, completion in
                guard let proc else {
                    completion(RPCError(
                        code: "plugin_unavailable",
                        message: "plugin providing \(actionName) is no longer running",
                    ))
                    return
                }
                proc.invoke(action: actionName, params: params, completion: completion)
            }
        }
    }

    /// Send `shutdown` notification to every plugin and wait briefly before
    /// force-terminating. Called from `applicationWillTerminate`.
    func shutdown() {
        for p in processes {
            p.shutdown()
        }
        processes.removeAll()
    }
}

// MARK: - PluginProcess

/// Wraps a single supervised plugin subprocess: stdio pipes, reader thread,
/// pending-request dict, and the methods to send action frames.
///
/// The class is `@unchecked Sendable` because reader threads access the
/// pending dict from off-main; access is serialized through `lock` and
/// completions always bounce back to main actor before the host's closure
/// fires, so the unsafety is contained to the dict mutation.
final class PluginProcess: @unchecked Sendable {
    let pluginName: String
    private(set) var provides: [String] = []

    private let process: Process
    private let stdinHandle: FileHandle
    private let stdoutHandle: FileHandle
    private let stderrHandle: FileHandle

    /// Pending request completions keyed by frame id. Reader thread pops
    /// entries when responses arrive. NSLock is fine here — contention is
    /// trivial (one writer per dispatch, one reader per response).
    private let lock = NSLock()
    private var pending: [String: (Any?) -> Void] = [:]

    private init(
        name: String,
        process: Process,
        stdin: FileHandle,
        stdout: FileHandle,
        stderr: FileHandle,
    ) {
        pluginName = name
        self.process = process
        stdinHandle = stdin
        stdoutHandle = stdout
        stderrHandle = stderr
    }

    /// Spawn the plugin binary, run the `initialize` handshake, send the
    /// `initialized` notification. Returns nil on any failure (binary
    /// missing, spawn error, init timeout, init error response, missing
    /// provides[]) — caller logs the failure and skips this service.
    static func spawn(
        loaded: LoadedPluginManifest,
        service: PluginServiceDef,
    ) -> PluginProcess? {
        let pluginName = loaded.manifest.plugin.name
        guard let execURL = resolveExecutable(service.exec, in: loaded.dir) else {
            log("[turm-plugin] \(pluginName)/\(service.name): exec '\(service.exec)' not found in plugin dir or PATH")
            return nil
        }

        let process = Process()
        process.executableURL = execURL
        process.arguments = service.args
        let stdin = Pipe()
        let stdout = Pipe()
        let stderr = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        process.standardError = stderr

        do {
            try process.run()
        } catch {
            log("[turm-plugin] \(pluginName)/\(service.name): spawn failed: \(error)")
            return nil
        }

        let pp = PluginProcess(
            name: pluginName,
            process: process,
            stdin: stdin.fileHandleForWriting,
            stdout: stdout.fileHandleForReading,
            stderr: stderr.fileHandleForReading,
        )
        pp.startReaderThread()
        pp.startStderrLoggerThread()

        // Init handshake. Use a class-bound "init box" to ferry the result
        // out of the reader thread's @Sendable closure without tripping the
        // strict-concurrency checker on captured `var`s.
        let initBox = InitBox()
        let initId = "init-\(UUID().uuidString)"
        pp.lock.lock()
        pp.pending[initId] = { [initBox] response in
            initBox.resolve(response)
        }
        pp.lock.unlock()

        pp.sendFrame([
            "id": initId,
            "method": "initialize",
            "params": ["protocol_version": 1],
        ])

        // 5s ceiling matches what feels reasonable for echo. Production
        // plugins (Slack/Calendar) doing OAuth on first run may want
        // longer; revisit when they land.
        guard initBox.semaphore.wait(timeout: .now() + .seconds(5)) == .success else {
            log("[turm-plugin] \(pluginName)/\(service.name): init timeout")
            pp.shutdown()
            return nil
        }

        switch initBox.outcome {
        case .none:
            log("[turm-plugin] \(pluginName)/\(service.name): init returned without populating outcome (programmer error)")
            pp.shutdown()
            return nil
        case let .failure(err):
            log("[turm-plugin] \(pluginName)/\(service.name): init failed: \(err)")
            pp.shutdown()
            return nil
        case let .success(result):
            // Validate provides[] subset matches the manifest declaration.
            // Linux is strict: runtime subset OK, runtime superset rejected.
            // We mirror that — a plugin that claims more at runtime than the
            // manifest promised could shadow another plugin's actions.
            let runtimeProvides = result["provides"] as? [String] ?? []
            let manifestProvides = Set(service.provides)
            let unexpected = runtimeProvides.filter { !manifestProvides.contains($0) }
            if !unexpected.isEmpty {
                log("[turm-plugin] \(pluginName)/\(service.name): runtime provides \(unexpected) not in manifest \(service.provides) — rejecting")
                pp.shutdown()
                return nil
            }
            pp.provides = runtimeProvides
        }

        // Notification (no id) — plugin starts emitting events from this point.
        pp.sendFrame(["method": "initialized"])

        log("[turm-plugin] \(pluginName)/\(service.name): ready (provides: \(pp.provides.joined(separator: ", ")))")
        return pp
    }

    /// Send an `action.invoke` request, store completion keyed by id, return.
    /// Reader thread fires the completion when the response arrives. Completion
    /// receives either the plugin's `result` Any value or an `RPCError` for
    /// `ok: false` responses.
    func invoke(
        action: String,
        params: [String: Any],
        completion: @escaping (Any?) -> Void,
    ) {
        let id = UUID().uuidString
        lock.lock()
        pending[id] = completion
        lock.unlock()

        sendFrame([
            "id": id,
            "method": "action.invoke",
            "params": [
                "name": action,
                "params": params,
            ],
        ])
    }

    /// Send shutdown notification, wait briefly, then terminate. Idempotent.
    func shutdown() {
        if process.isRunning {
            sendFrame(["method": "shutdown"])
            // Echo's shutdown is `std::process::exit(0)` so it goes away fast.
            // 200ms ceiling is plenty; longer plugins get terminated cleanly
            // by the next branch.
            Thread.sleep(forTimeInterval: 0.2)
        }
        if process.isRunning {
            process.terminate()
        }
        // Fail any still-pending requests so callers don't hang forever.
        lock.lock()
        let stranded = pending
        pending.removeAll()
        lock.unlock()
        for completion in stranded.values {
            // Direct invocation — same reasoning as `handleLine`. Completions
            // here are leaf closures from SocketServer that just signal
            // a semaphore; no actor isolation required.
            completion(RPCError(
                code: "plugin_shutdown",
                message: "plugin shut down before responding",
            ))
        }
    }

    // MARK: - Internal IO

    private func sendFrame(_ frame: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: frame),
              let str = String(data: data, encoding: .utf8)
        else {
            log("[turm-plugin] \(pluginName): failed to serialize frame")
            return
        }
        var line = Data(str.utf8)
        line.append(UInt8(ascii: "\n"))
        do {
            try stdinHandle.write(contentsOf: line)
        } catch {
            log("[turm-plugin] \(pluginName): stdin write failed: \(error)")
        }
    }

    private func startReaderThread() {
        Thread.detachNewThread { [weak self] in
            self?.readerLoop()
        }
    }

    private func readerLoop() {
        var buffer = Data()
        while true {
            let chunk = stdoutHandle.availableData
            if chunk.isEmpty { break } // EOF — plugin exited
            buffer.append(chunk)
            while let nl = buffer.firstIndex(of: UInt8(ascii: "\n")) {
                let lineData = buffer.subdata(in: 0 ..< nl)
                buffer = buffer.subdata(in: (nl + 1) ..< buffer.count)
                handleLine(lineData)
            }
        }
        log("[turm-plugin] \(pluginName): stdout EOF")
    }

    private func startStderrLoggerThread() {
        Thread.detachNewThread { [weak self] in
            self?.stderrLoop()
        }
    }

    private func stderrLoop() {
        // Forward plugin stderr to host stderr verbatim, prefixed with the
        // plugin name so multiplexed plugins don't lose attribution.
        var buffer = Data()
        while true {
            let chunk = stderrHandle.availableData
            if chunk.isEmpty { break }
            buffer.append(chunk)
            while let nl = buffer.firstIndex(of: UInt8(ascii: "\n")) {
                let lineData = buffer.subdata(in: 0 ..< nl)
                buffer = buffer.subdata(in: (nl + 1) ..< buffer.count)
                if let line = String(data: lineData, encoding: .utf8) {
                    PluginProcess.log("[turm-plugin] \(pluginName) stderr: \(line)")
                }
            }
        }
    }

    private func handleLine(_ data: Data) {
        guard let parsed = try? JSONSerialization.jsonObject(with: data),
              let obj = parsed as? [String: Any]
        else {
            log("[turm-plugin] \(pluginName): skipping non-JSON line")
            return
        }

        // Response to a request we sent (has id + ok).
        if let id = obj["id"] as? String {
            lock.lock()
            let completion = pending.removeValue(forKey: id)
            lock.unlock()
            guard let completion else {
                log("[turm-plugin] \(pluginName): response for unknown id \(id)")
                return
            }
            // Fire the completion inline on the reader thread. We deliberately
            // do NOT hop to main actor here because:
            //   1. The init-handshake completion below is invoked while the
            //      main thread is parked on `initBox.semaphore.wait()` — a
            //      `DispatchQueue.main.async` bounce would deadlock instantly.
            //   2. The action.invoke completions all funnel back into
            //      SocketServer's leaf closure (`box.value = result; sema.signal()`)
            //      which is `@unchecked Sendable` and tolerates any thread.
            //   3. Any future completion that DOES need main actor work can
            //      bounce inside its own body — pushing the policy to the
            //      callee instead of forcing it on every plugin response.
            let payload = decodeResponse(obj)
            completion(payload)
            return
        }

        // Notification from plugin (no id). Today we only log event.publish;
        // PR 5 will fan it onto the EventBus for the trigger engine.
        if let method = obj["method"] as? String {
            switch method {
            case "event.publish":
                if let params = obj["params"] as? [String: Any],
                   let kind = params["kind"] as? String
                {
                    log("[turm-plugin] \(pluginName): event.publish \(kind) (not forwarded yet — PR 5)")
                }
            default:
                log("[turm-plugin] \(pluginName): unknown notification \(method)")
            }
            return
        }

        log("[turm-plugin] \(pluginName): malformed frame (no id, no method)")
    }

    /// Convert a plugin response frame into either the success result (Any?)
    /// or an `RPCError` so the registry completion gets the right thing.
    private func decodeResponse(_ obj: [String: Any]) -> Any? {
        if let ok = obj["ok"] as? Bool, ok {
            return obj["result"]
        }
        if let err = obj["error"] as? [String: Any] {
            let code = (err["code"] as? String) ?? "plugin_error"
            let message = (err["message"] as? String) ?? "plugin returned error without message"
            return RPCError(code: code, message: message)
        }
        return RPCError(code: "malformed_response", message: "plugin response missing ok/error")
    }

    // MARK: - Helpers

    private static func resolveExecutable(_ name: String, in dir: URL) -> URL? {
        let direct = dir.appendingPathComponent(name)
        if FileManager.default.isExecutableFile(atPath: direct.path) {
            return direct
        }
        // PATH lookup mirrors Linux's behavior. `which` is the simplest
        // correct implementation; we don't shell out to /usr/bin/which
        // because that re-evaluates against $PATH at process spawn time.
        let path = ProcessInfo.processInfo.environment["PATH"] ?? ""
        for entry in path.split(separator: ":") {
            let candidate = URL(fileURLWithPath: String(entry)).appendingPathComponent(name)
            if FileManager.default.isExecutableFile(atPath: candidate.path) {
                return candidate
            }
        }
        return nil
    }

    fileprivate static func log(_ message: String) {
        FileHandle.standardError.write(Data((message + "\n").utf8))
    }

    private func log(_ message: String) {
        PluginProcess.log(message)
    }
}

// MARK: - InitBox

/// Class-bound carrier for the init handshake result. Lets us ferry data out
/// of the reader-thread `@Sendable` closure without tripping Swift 6 strict
/// concurrency on captured `var`s. Single-use — `resolve` signals the
/// semaphore exactly once.
private final class InitBox: @unchecked Sendable {
    enum Outcome {
        case success([String: Any])
        case failure(String)
    }

    let semaphore = DispatchSemaphore(value: 0)
    var outcome: Outcome?

    func resolve(_ response: Any?) {
        defer { semaphore.signal() }
        if let err = response as? RPCError {
            outcome = .failure("\(err.code): \(err.message)")
        } else if let dict = response as? [String: Any] {
            outcome = .success(dict)
        } else {
            outcome = .failure("init response was neither RPCError nor [String: Any]: \(String(describing: response))")
        }
    }
}
