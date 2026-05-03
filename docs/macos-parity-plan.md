# turm macOS ↔ Linux Parity Plan (v2 — codex round-1 reflected)

## Goal

Bring the macOS app to feature/UX parity with Linux. Phase 1 MVP, WebView panel, AI Agent / event integration, and config hot-reload already shipped on macOS. This plan covers everything still missing, prioritized by user-visible impact and architectural dependency.

## Confirmed-divergent areas (audit summary)

Source-of-truth audit of the working tree at HEAD (`master`):

- **Socket commands**: Linux 45 / macOS 32. Missing on macOS:
  `background.next`, `background.toggle`, `claude.start`, `theme.list`,
  `plugin.list`, `plugin.open`, `plugin.<service>` (generic dispatcher), `statusbar.show`/`hide`/`toggle`,
  `webview.screenshot`, `webview.query`, `webview.query_all`, `webview.get_styles`, `webview.click`, `webview.fill`, `webview.scroll`, `webview.page_info`.
  Per-method `id` parameter for webview commands is required by Linux but ignored by macOS (active-panel only). Parity = adopt panel `id` resolution on macOS.
- **Service plugin host**: `turm-linux/src/service_supervisor.rs` (1794 LOC). macOS has no host. Plugins gated `target_os = "linux"` (kb/todo/bookmark) or `unix` (calendar/slack/llm).
- **Trigger engine + Action Registry + ContextService + condition DSL**: live in `turm-core`. Linux pumps via `LiveTriggerSink` (`window.rs`). macOS uses only `EventBus.swift`. **Trigger engine in particular carries non-trivial semantics**: await preflight & pending state (Phase 14.2 `<trigger>.awaited` synthesis), completion/failure fanout, `covering_patterns` subscription dedup, reload draining preserving in-flight events, ordering guarantees. Underestimating this in v1 broke Slack→todo / `slack.get_message` chained workflows.
- **Status bar**: `turm-linux/src/statusbar.rs` (407 LOC, Waybar-style modules); none on macOS.
- **Plugin panels (HTML/JS bridge)**: `turm-linux/src/plugin_panel.rs` (418 LOC) hosts WebKit-rendered panels with a **reply-capable** bridge — JS calls `await window.turm.call(method, params)` and receives a structured response. macOS has no equivalent.
- **Custom keybindings**: Linux parses `[keybindings]` and runs `spawn:<cmd>` as **out-of-band shell processes** via `spawn_command` (`tabs.rs:1413`), NOT via the socket dispatcher. macOS shortcuts are hardcoded.
- **Pane focus navigation**: Linux Ctrl+Shift+N/P/arrows; macOS missing.
- **Tabs position**: Linux supports top/bottom/left/right; macOS pins top.
- **Background random rotation**: Linux reads a flat list at `~/.cache/terminal-wallpapers.txt` (note: docs claim `~/.cache/turm/wallpapers.txt` — docs lag, source const is `terminal-wallpapers.txt`). Populating that list (e.g. from a `[background] directory`) is itself unimplemented on Linux. Active mode toggle is a separate file. macOS has none of this.
- **Terminal-side gaps**: OSC 52 clipboard and clickable URL detection pending on both, but **see C1 below: macOS already permits unconditional OSC 52 writes through SwiftTerm**.

## Acknowledged irreducible gap

- **`terminal.output` event**: SwiftTerm's `feed(byteArray:)` is an extension method, not overridable. Documented in `docs/macos-app.md`. Out of scope.

---

## Codex round-1 findings reflected (verified against the source)

### CRITICAL

- **[C1] OSC 52 is a security regression on macOS today, not a future feature.**
  `SwiftTerm/Mac/MacLocalTerminalView.swift:107` declares `public func clipboardCopy(source:content:)` that calls `NSPasteboard.general.writeObjects([str])` unconditionally. The method is `public`, **not `open`** — we cannot override it from outside SwiftTerm, and it does not forward through `processDelegate`. So **today**, any program in a turm pane can write to the macOS pasteboard via OSC 52 with no user gesture and no off-switch. Plan changes:
  - Treat OSC 52 as a **blocker for Tier 1**, not a parity item.
  - Fix path: replace `LocalProcessTerminalView` with our own subclass that owns being the `TerminalDelegate` and proxies `clipboardCopy` ourselves (gated on `[security] osc52 = "deny" | "ask" | "allow"`, default `"deny"`). If subclassing turns out blocked by the same `public`-not-`open` issue on the upstream class, we fork SwiftTerm. Either way it's a real engineering chunk, not a delegate stub.
  - Linux's VTE has the same audit: `vte4::Terminal` exposes `set_enable_osc_52` (or equivalent); confirm our config gate applies symmetrically.

- **[C2] Trigger engine is not "small bookkeeping" — re-implementing it in Swift will silently break chained workflows.**
  `turm-core/src/trigger.rs:154` plus `turm-linux/src/window.rs:551` show: `<trigger_name>.awaited` synthesis for `event.subscribe`-style chains, completion/failure fanout to `LiveTriggerSink::dispatch_action` consumer, `covering_patterns` dedup so overlapping globs collapse to one bus receiver, hot-reload `reconcile()` that **preserves still-needed receivers' pending events**, and per-event ordering. Plan changes:
  - **Drop "Option A — reimplement engines in Swift" from Tier 2.** Tier 2 splits as:
    - Native Swift: `ContextService` (small data class, polled from main runloop), thin `ActionRegistry` shim that maps method name → completion handler.
    - Rust FFI (cdylib + C-ABI): `TriggerEngine`, `condition` DSL, `event_bus` covering/subscriber bookkeeping. JSON in / JSON out at the boundary.
  - This means the FFI build-system work (cargo + SPM build phase, static `.a` bundled in `Turm.app`) lands in Tier 2, not Tier 3. Tier 3 (supervisor) reuses the same FFI pipeline.
  - **De-risk first**: a small Tier 2 spike that exposes only `TriggerEngine::set_triggers` + one awaited-trigger fan-out, validated end-to-end with `event.subscribe`, before committing to wider FFI surface.

- **[C3] Plugin panel JS bridge needs a reply-capable handler.**
  `plugin_panel.rs:207` exposes `await window.turm.call(method, params)` returning a structured response. Plain `WKScriptMessageHandler` is fire-and-forget. Plan change:
  - Use `WKScriptMessageHandlerWithReply` (macOS 11+, well within our `macOS 14+` target).
  - Public JS contract (`window.turm.call`) stays unchanged; the injected user-script glue handles the request-id round-trip. **No fallback to `postMessage` + manual id**; the API existed in our minimum target.

### IMPORTANT

- **[I1] Webview commands need panel `id` resolution on macOS.**
  Linux's `webview.navigate/back/forward/reload/query/click/fill/scroll/state` take an `id` param. macOS targets `activeWebView`. AI/web automation parity (concurrent panels, async commands) requires panel-id resolution. Plan change:
  - Add stable per-panel ids to `TurmPanel` (UUID at creation), expose `getPanel(id)` on `TabViewController`, branch in command handlers: id present → resolve; absent → fall back to active.
  - Already needed for Tier 4 webview interaction commands; bring it forward to Tier 1 #5 (URL click) so we don't ship the easy commands with active-only and then change the contract later.

- **[I2] Background rotation is `~/.cache/terminal-wallpapers.txt` + active-mode flag, not `[background] directory`.**
  Linux reads a single flat cache file populated externally (not by turm). The active/deactive toggle is a separate file. The roadmap entry "background random rotation + `[background] directory`" is forward-looking and not yet implemented on **either** platform. Plan change:
  - Drop "macOS-side `[background] directory` config" from Tier 1.
  - macOS parity = read the same `~/.cache/terminal-wallpapers.txt` (it's a Linux-y path; on macOS use `~/Library/Caches/turm/wallpapers.txt` AND read the legacy path as fallback for users who already have it). `background.next` = pick a random line. `background.toggle` = same active-mode file semantics.
  - If we want directory-driven population, that's a separate roadmap item that lands on **both** platforms behind a shared `turm-core::background` module.

- **[I3] `Config.swift`'s hand-rolled parser dies on `[keybindings]` + `[[triggers]]` + `[security]`.**
  Linux config already requires arrays-of-tables (`[[triggers]]`), inline tables (condition strings with embedded quotes), escaped strings. Plan change:
  - Swap `Config.swift` to a real TOML parser (SwiftPM dep `swift-toml` or `TOMLDecoder`) **before Tier 1 #2**. It's a prerequisite, not a Tier-2 task.
  - Round-trip-test against the existing `examples/plugins/*/triggers.example.toml` files.

- **[I4] Custom keybindings spawn shell commands out-of-band, not via socket dispatch.**
  Linux's `[keybindings]` map `"ctrl+shift+g" = "spawn:~/script.sh --arg"` to `spawn_command(&binding.command)` — a fork/exec, not a `socket::dispatch` call. Plan change:
  - macOS keybindings use `Process` (or `posix_spawn`) with `TURM_SOCKET` env injection, exactly like Linux.
  - **Trigger-action-via-keybinding** (the "Custom-keybinding spawn" Tier 2 item) is a *separate* feature: an opt-in `"action:webview.open ..."` syntax that DOES go through ActionRegistry. Both syntaxes are needed for parity (`spawn:`) plus useful extension (`action:`).
  - Document the security implication: `spawn:` runs commands with the user's full env. Same as Linux.

- **[I5] Supervisor shutdown needs multiple paths + process-group ownership.**
  Linux uses `window.connect_destroy` + `glib::unix_signal_add_local` (SIGTERM/SIGINT). macOS app termination is best-effort; SIGKILL bypasses everything. Plan change:
  - On macOS: spawn each plugin in its **own process group** (`setpgid(0,0)` in a `pre_exec`-equivalent on the child). Track child PIDs in a file at `~/Library/Caches/turm/plugin-pids.json`.
  - Shutdown hooks: `applicationShouldTerminate` → graceful, `applicationWillTerminate` → SIGTERM the group, signal handlers (SIGTERM/SIGINT) → same.
  - **Crash recovery**: at next launch, read the PID file; for any pid still alive whose pgid we own, SIGKILL the group. This is the only mitigation for SIGKILL'd parents.

### NICE-TO-HAVE

- **[N1] Source comment in `TerminalViewController.swift:10` claims PTY output interception; reality is `dataReceived` is not implemented in the subclass.** Update comment to match the documented irreducible gap.
- **[N2] SwiftTerm has `requestOpenLink` for OSC 8 hyperlinks.** Use that for explicit links; only fall back to regex-on-buffer for plain-text URLs (regex hit-testing across wrapped lines / scrollback / alt screen is fragile).

---

## Updated phased plan

### Tier 0 — Pre-requisites (must land before Tier 1)

- **0.1 Real TOML parser in `Config.swift`** (I3) ✅ — SwiftPM dep `LebJe/TOMLKit` (0.6.0). `TurmConfig.parse` decodes via `TOMLDecoder` into a private `RawConfig` shadow type whose sections (`[terminal]`, `[theme]`, `[background]`, `[security]`) are all-optional so any subset (including the empty file) round-trips to defaults. Snake-case keys (`font_family`, `font_size`) handled with manual `CodingKeys` since TOMLKit 0.6 has no `keyDecodingStrategy`. Unknown sections from the Linux schema (`[tabs]`, `[statusbar]`, `[keybindings]`, `[[triggers]]` with nested inline tables) parse without error and are silently dropped — confirmed against a stand-in test that loads the user's existing `~/.config/turm/config.toml` plus a synthetic full-Linux-shape config. Malformed TOML logs a `[turm] config.toml parse failed: …` line to stderr and falls through to defaults instead of crashing.
- **0.2 Stable panel ids on `TurmPanel`** (I1 prep) ✅ — `TurmPanel.panelID` (UUID) was already present on `TerminalViewController` and `WebViewController`. Surfaced `TabViewController.panel(id:) -> (any TurmPanel)?` and `webView(id:) -> WebViewController?` that walk every tab's split tree (O(N) but N is small in practice). Adopted by Tier 1.6 webview command resolution.
- **0.3 OSC 52 deny-by-default** (C1) ✅ — `TurmTerminalDelegate` proxy owns SwiftTerm's `terminalDelegate` slot, gates `clipboardCopy` on `[security] osc52` (default `"deny"`, opt-in `"allow"`). Hot-reload via `applyOSC52Policy`. VTE on Linux is already deny-by-default, so this fix is macOS-only. Tri-state plan deferred — `"ask"` requires modal-on-PTY-thread UX design; ship binary deny/allow first.

### Tier 1 — UX parity

1. **Pane focus navigation** — Cmd+Shift+] / Cmd+Shift+[ + arrow variants in `PaneManager`. DFS over SplitNode.
2. **Custom keybindings** — `[keybindings]` parsed via Tier 0 TOML lib. Two syntaxes: `spawn:<cmd>` (out-of-band Process, env includes `TURM_SOCKET`) and `action:<method> [params]` (registry dispatch). Intercept via `NSEvent.addLocalMonitorForEvents`. Built-ins still hardcoded; custom checked first.
3. **Background random rotation** — Read `~/Library/Caches/turm/wallpapers.txt` (with `~/.cache/terminal-wallpapers.txt` fallback). `background.next` and `background.toggle` socket commands. Active-mode flag mirrors Linux file-based toggle.
4. **Tabs position (top/bottom)** — `[tabs] position`, swap Y-anchor in `TabViewController`. `left`/`right` deferred.
5. **URL detection + click-to-open** — SwiftTerm's `requestOpenLink` for OSC 8; regex fallback for plain text. Use Tier 0.2 panel id when emitting events.
6. **Webview command panel-id parity** (I1) ✅ — All `webview.*` commands (`navigate` / `back` / `forward` / `reload` / `execute_js` / `get_content` / `devtools` / `state`) now resolve via `params["id"]` first and fall back to active webview when absent (lenient default per plan; Linux requires `id`). Errors use a new `RPCError` sentinel that `SocketServer.dispatch` detects and maps to a proper JSON-RPC error envelope (`{ok:false, error:{code, message}}`) — codes match Linux: `not_found` / `wrong_panel_type` / `invalid_params` / `no_active_webview`. Two wire-protocol fixes landed alongside: `execute_js` reads `code` (Linux + turm-cli convention) with `script` as back-compat fallback; `devtools` accepts `action: show/close/attach/detach/toggle` (macOS WKWebView has no public inspector-open API, so all enable-style verbs map to the existing `developerExtrasEnabled` toggle and `close` is a no-op — protocol parity without functional parity for the inspector window). Success results standardized to `{status: "ok"}` matching `webview.rs`.

### Codex round-2 reordering (plugin parity)

Round-2 codex pressure-test (against the post-Tier-0/1 working tree) ruled that **Tier 2→3 is the wrong order if the goal is "working KB/Slack/Todo fastest"** — `TriggerEngine` only calls a `TriggerSink`, so without registered plugin actions a trigger spike still leaves every user-facing plugin command returning `unknown_method`. New 5-PR sequence:

1. **PR 1: FFI build/link spike only** ✅ — `turm-ffi` staticlib (`turm_ffi_version` + `turm_ffi_call_json` + `turm_ffi_free_string` + `turm_ffi_last_error`), CTurmFFI clang module wrapper, SwiftPM linker settings, build-script wrappers, smoke test in `applicationDidFinishLaunching` confirming JSON round-trip with Rust-generated timestamp. arm64-only, cargo-built statically. No engine, no plugins — just proves the boundary works. Surface kept tiny on purpose.
2. **PR 2: macOS registry-first dispatch seam** ✅ — `ActionRegistry.swift` (`@MainActor`, minimal surface: `register` / `tryDispatch` / `has` / `names` / `count`) mirrors Linux's `try_dispatch` bool-return semantic so callers compose the same way. `AppDelegate.handleCommand` checks `actionRegistry.tryDispatch(...)` first; on `false` (not registered) it falls through to the legacy hardcoded switch — no behavior change for existing methods. `system.ffi_test` and `system.list_actions` registered on launch as proof-of-seam: `turmctl call system.ffi_test --params '{...}'` walks turmctl → unix socket → SocketServer.dispatch → main thread → handleCommand → ActionRegistry.tryDispatch → TurmFFI.callJSON → Rust `extern "C"` round-trip → back through every layer; `system.list_actions` returns `{count, names}` for diagnostics. Unknown-method path still returns `{ok:false, error:{code:"unknown_method", message:"unknown: <method>"}}` via the same RPCError envelope wired in Tier 1.6. **Deferred from Linux's full surface:** `register_silent` (no completion bus on macOS yet — every registered action is effectively silent today), `register_blocking` (codex round-2 flagged that macOS's socket-thread-blocking model conflicts with Linux's worker-thread spawn — the trigger engine in PR 5 will redesign the async boundary holistically before re-adding), `invoke` / `try_invoke` sync paths (only the trigger engine needs them).
3. **PR 3: one service plugin host before triggers** — start with `turm-plugin-echo` (NOT KB/Slack), prove manifest discovery + spawn + init handshake + `echo.ping` round-trip + shutdown + Linux-shape error envelope through `turmctl call echo.ping`.
4. **PR 4: port one real direct-action plugin** — `git` or `llm.auth_status` first (no Linux-only filesystem primitives). KB/Todo/Bookmark are deferred until their atomic-create / `O_NOFOLLOW` / `renameat2` backends are replaced — they currently won't even compile on macOS.
5. **PR 5: event-producing plugin OR trigger engine** — Slack/Calendar if their `keyring`/`apple-native` Keychain path works; otherwise TriggerEngine FFI against a native test action (don't call that parity).

Open spike questions still owed: SwiftPM cargo-prebuild robustness (PR 1 settled this with the wrapper-script approach — see Tier 2.1 below), universal-binary policy, Gatekeeper/quarantine on plugin binaries, macOS-safe atomic-create replacement for KB/Todo/Bookmark, `keyring` Keychain prompt UX from unsigned plugin binaries.

### Tier 2 — Wire turm-core engines on macOS

- **2.1 FFI scaffolding** ✅ (PR 1) — `turm-ffi` crate (`crate-type = ["staticlib"]`, depends only on `serde_json`) lives at workspace root, exposes 4 `extern "C"` symbols documented in `turm-ffi/src/lib.rs`. Built via `cargo build --release -p turm-ffi` → `target/release/libturm_ffi.a`. SwiftPM consumes through a `CTurmFFI` C target (`Sources/CTurmFFI/include/{turm_ffi.h,module.modulemap}` + a `dummy.c` so SPM emits an object file that carries the linker settings); the `Turm` executable target adds `-L../target/release -lturm_ffi` via `linkerSettings`. The cargo step cannot be invoked from `Package.swift` directly in SwiftPM 6.0, so `scripts/install-macos.sh` and `turm-macos/run.sh` both run `cargo build --release -p turm-ffi` before `swift build`. arm64 only for now (host-arch cargo default); universal-binary recipe is `cargo build --target aarch64-apple-darwin && cargo build --target x86_64-apple-darwin && lipo -create … -output …`, deferred until there's a real x86_64 user. Smoke test in `AppDelegate.applicationDidFinishLaunching` calls `turm_ffi_version()` and `turm_ffi_call_json()`, prints to stderr — verified end-to-end (`[turm-ffi] echo round-trip = ["spike": 1, "hello": from swift, "echoed_at": <unix-ms>]`).
- **2.2 ContextService (Swift native)** — `active_panel`, `active_cwd`, per-panel cwd cache. Polled from main runloop (`DispatchSourceTimer`).
- **2.3 ActionRegistry (Swift native, thin)** ✅ (PR 2) — pulled forward from Tier 2.3 to PR 2 because plugin host and trigger engine both need a registry seam. See PR 2 description above for what shipped. `register_blocking` and silent variants intentionally deferred until PR 5's trigger engine forces the async-boundary redesign.
- **2.4 TriggerEngine via FFI** (C2) — Rust core does compilation, dedup, await synthesis, reload reconcile. Swift adapter forwards events from `EventBus.swift` into the engine and receives action-dispatch callbacks. Threading: a single serial queue owns the engine handle; main-thread snapshots context just-in-time. Validated by an end-to-end `event.subscribe` chained-trigger test. **Now PR 5** (after plugin host so the engine has real actions to dispatch).

### Tier 3 — Service plugin host on macOS

- **3.1 Supervisor via Rust FFI** — extract supervisor into `turm-supervisor` crate (or include in `turm-ffi` from Tier 2.1). Reuses 7 existing unit tests + Phase 9.5 forwarder-leak fix + restart policy.
- **3.2 Process-group ownership + shutdown matrix** (I5) — `setpgid(0,0)` per child, PID file at `~/Library/Caches/turm/plugin-pids.json`. Hook into `applicationShouldTerminate` / `applicationWillTerminate` / signal handlers. Crash-recovery sweep at next launch.
- **3.3 Plugin platform gates** — flip `cfg(not(target_os = "linux"))` to `cfg(not(any(target_os = "linux", target_os = "macos")))` on `turm-plugin-{kb,todo,bookmark}`. `keyring` already falls through to Keychain.
- **3.4 Plugin discovery** — `~/Library/Application Support/turm/plugins/<name>/` (note: NOT `~/.config/...` on macOS — match platform conventions). Support `~/.config/turm/plugins/<name>/` as a secondary path so users sharing dotfiles across Linux/macOS work.
- **3.5 `install-plugins-macos.sh`** — handles macOS code-signing for first-party plugin binaries (or document the unsigned-binary Gatekeeper bypass).

### Tier 4 — Plugin panels + status bar + remaining socket commands

- **4.1 Plugin panel** (C3) — `PluginPanelController : TurmPanel` wrapping `WKWebView` with `WKScriptMessageHandlerWithReply`. Mirror Linux's `window.turm.call(method, params) → Promise` contract verbatim.
- **4.2 Status bar** — Swift `StatusBarView : NSView`. `[statusbar.module]` config sections, per-module `update_interval_secs` + `output` shell command. `statusbar.show/hide/toggle` socket commands.
- **4.3 Remaining socket commands** — `theme.list`, `plugin.list`, `plugin.open`, `plugin.<x>` generic dispatcher, `claude.start`. Webview interaction (`webview.query`, `webview.click`, `webview.fill`, `webview.scroll`, `webview.screenshot`, `webview.get_styles`, `webview.page_info`) via `WKWebView.evaluateJavaScript`, return shapes match Linux.

### Tier 5 — Cross-platform roadmap items (out-of-parity-scope)

Session persistence, command palette, KB FTS5, deferred LLM/Slack/Discord phases — neither platform has them; not parity items.

---

## Codex's answers to the 6 architectural questions (paraphrased, kept)

1. **Engine split** — Don't reimplement TriggerEngine in Swift; ContextService and a thin action-registration layer can be native, but trigger semantics MUST come from Rust FFI or the macOS version drifts immediately on await/completion behavior. **Adopted in v2 plan.**
2. **Supervisor lifecycle** — Hook `applicationShouldTerminate` + `applicationWillTerminate` + window close + signal handlers; for SIGKILL the only mitigation is process-group ownership + PID-file crash-recovery sweep at next launch. **Adopted in I5 / Tier 3.2.**
3. **MainActor safety** — Don't put the engine itself behind `@MainActor`. Small main-thread adapter snapshots UI context, forwards serial events into an isolated engine queue. **Adopted in Tier 2.4.**
4. **OSC 52 trust model** — Won't ship via a `LocalProcessTerminalViewDelegate` method (delegate path doesn't exist for clipboard); requires replacing/proxying `terminalDelegate` or forking SwiftTerm. **Adopted as Tier 0.3 blocker.**
5. **Plugin bridge** — Use `WKScriptMessageHandlerWithReply` (available on our minimum target). **Adopted in C3 / Tier 4.1.**
6. **Phasing** — Land Tier 0 + Tier 1 (architecture-orthogonal). For Tier 2, do a small Rust-FFI spike (one awaited trigger end-to-end) BEFORE committing to wider FFI surface. **Adopted; Tier 2.1+2.4 explicitly carry the spike-first ordering.**

## Out of scope (explicit)

- `terminal.output` PTY interception (SwiftTerm public-API limit).
- D-Bus integration (Linux-only; macOS uses Unix socket only — already correct).
- Tabs position `left`/`right` (low ROI; defer).
- Wayland/GTK-specific behaviors that don't translate.
