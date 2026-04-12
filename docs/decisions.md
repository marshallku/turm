# Technical Decisions

## 1. Tauri v2 Abandoned → Native Platform UIs

**Problem:** Tauri IPC introduced noticeable input latency in the terminal. Every keypress went through JS → Tauri invoke → Rust → PTY, and output went PTY → Rust → Tauri event → JS → xterm.js. The round-trip was perceptible.

**Decision:** Switched to platform-native UIs with a shared Rust core:

- Linux: GTK4 + VTE4 (VTE handles PTY internally, zero IPC overhead)
- macOS: Swift/AppKit (SwiftTerm or Ghostty embedding, TBD)

**Tradeoff:** More code per platform, but terminal responsiveness is non-negotiable.

## 2. VTE Handles PTY on Linux

**Rationale:** VTE has its own optimized PTY management. Using `portable-pty` alongside VTE would mean double PTY handling. Let VTE do what it does best.

**Consequence:** `turm-core/pty.rs` and `state.rs` were removed — both platforms handle PTY natively (VTE on Linux, SwiftTerm on macOS).

## 3. D-Bus for Linux IPC (Not Unix Socket)

**Rationale:** D-Bus is the standard Linux IPC mechanism. Using it means:

- No custom socket server needed
- System integration (other tools can control turm)
- Session bus handles lifecycle automatically

**GTK thread safety issue:** GTK widgets are not `Send+Sync`. D-Bus callbacks can't directly modify widgets.

**Solution:** `mpsc::channel` + `glib::timeout_add_local(50ms)` polling on the GTK main thread. D-Bus handler sends commands through the channel, GTK main loop polls and applies them.

**Note:** `glib::MainContext::channel` was removed in newer glib versions, so we use `std::sync::mpsc` with manual polling instead.

## 4. GtkOverlay for Background Compositing

**Stack:** `bg_picture` (child) → `tint_overlay` (overlay) → `terminal` (overlay)

**Critical detail:** VTE paints its own opaque background by default. To see the image layers beneath, you must:

1. Call `terminal.set_clear_background(false)`
2. Set VTE background color to transparent `RGBA(0,0,0,0)`

Without step 1, VTE covers the entire overlay with its own background color.

## 5. Binary Names: turm + turmctl

**Problem:** Both turm-linux and turm-cli had `[[bin]] name = "turm"`, causing Cargo output filename collision.

**Decision:** CLI binary renamed to `turmctl` (follows kubectl, sysctl naming convention).

## 6. Theme System

**Design:** Themes are defined as `Theme` structs in `turm-core/theme.rs` with semantic color slots (foreground, background, 16-color palette, surface/overlay/accent UI colors). 10 built-in themes are embedded. All UI components (terminal, tab bar, search bar, webview URL bar, window background) use theme colors via CSS generation functions.

**Config:** `[theme] name = "catppuccin-mocha"` selects the active theme. Hot-reloads on config change.

**Built-in themes:** catppuccin-mocha (default), catppuccin-latte, catppuccin-frappe, catppuccin-macchiato, dracula, nord, tokyo-night, gruvbox-dark, one-dark, solarized-dark.

## 7. cmux V2 Protocol for Socket Communication

**Format:** Newline-delimited JSON with UUID request IDs.
**Reference:** ~/dev/cmux/ (Marshall's macOS terminal multiplexer)

This protocol is used by both turmctl and the turm-linux socket server. D-Bus remains for system integration (background control), while the socket API handles all rich control (tabs, splits, webview, terminal agent, approval workflow).

## 8. Forced Dark Theme

**Problem:** When VTE background is transparent (for bg images) and no image is loaded yet, the system GTK theme shows through. On light themes this makes the terminal white.

**Fix:** Force dark theme in `app.rs` via `set_gtk_application_prefer_dark_theme(true)` + CSS `window { background-color: #1e1e2e; }`.

## 9. Rust Edition 2024

Using the latest Rust edition. No compatibility concerns since the project is new.

## 10. In-Terminal Search via VTE Regex

**Problem:** Popular terminals (Ghostty, Kitty) lack built-in Ctrl+F search, requiring piping through external tools.

**Decision:** Implemented search using VTE4's built-in `search_set_regex` / `search_find_next` / `search_find_previous` with PCRE2 regex. Search bar is a `gtk4::Box` overlay at the bottom of each terminal panel.

**UX details:**

- Search text is preserved when closing, but fully selected on reopen (type to replace, Enter to reuse)
- `glib::idle_add_local_once` is needed for `select_region` — GTK4 Entry ignores selection before focus is fully settled

## 12. macOS Split Panes: NSSplitViewDelegate for Equal Initial Sizing

**Problem:** Getting `NSSplitView` to start at exactly 50/50 on initial layout is unreliable. Two failed approaches:

1. `DispatchQueue.main.asyncAfter(deadline: .now() + 0.05)` + `setPosition`: timing is unpredictable. The timer may fire before layout resolves (position ignored) or after a subsequent split has already started.
2. `override func layout()` + `setPosition`: NSSplitView calls `resizeSubviews` (which commits subview frames) before calling `layout()`. By the time `layout()` fires, the wrong frames are already in place.

**Decision:** Use `NSSplitViewDelegate.splitView(_:resizeSubviewsWithOldSize:)`. This delegate method is the exact hook where NSSplitView asks "how should I size my subviews?" — set frames directly here. An `initialSizeSet` flag ensures this only runs once per `EqualSplitView` instance; subsequent calls fall back to `adjustSubviews()` to allow user dragging.

## 13. macOS Split Panes: Hierarchical (Not Flat) Splitting

**Problem:** When splitting a pane that is already part of a split, two approaches are possible:

- **Flat:** Add the new pane as a sibling in the parent branch → all siblings resize equally. If you have [A|B] and split A, result is [A|newPane|B] with each pane at 33%.
- **Hierarchical:** Replace A's leaf with a new 2-child branch → only A's space is divided. If you have [A|B] and split A, result is [(A|newPane)|B] with A and newPane each at 25%, B untouched at 50%.

**Decision:** Always use hierarchical splitting. The flat approach is surprising because splitting one pane causes other panes to shrink. "Split this pane in half" is a more intuitive mental model than "add a pane to this group."

**Implementation:** `SplitNode.splitting(_:with:orientation:)` always wraps the target leaf in a new 2-child branch, regardless of the parent branch's orientation. `removing(_:)` collapses a branch to its single remaining child when a pane is closed.

## 14. macOS: Async Socket Handler via DispatchSemaphore + ResultBox

**Problem:** Some socket commands (e.g. `webview.execute_js`, `webview.get_content`) get their results from WKWebView callbacks, which run on the main thread asynchronously after the initial dispatch. The socket thread needs to block until the result is available.

**Decision:** Changed `SocketServer.commandHandler` from a synchronous `(method, params) -> Any?` signature to a completion-based `(method, params, completion: (Any?) -> Void) -> Void`. The socket thread blocks on a `DispatchSemaphore`. The main thread calls completion (possibly from a WKWebView callback), which stores the value in a `ResultBox: @unchecked Sendable` and signals the semaphore.

**Why `ResultBox`:** Swift 6 strict concurrency rejects capturing a `var` local in an `@MainActor` closure sent to another thread. A `final class` box with `@unchecked Sendable` is safe because the semaphore serializes all access — the socket thread never reads until after the signal.

## 15. macOS: TurmPanel Protocol for Mixed Terminal+WebView Splits

**Problem:** `SplitNode` and `PaneManager` were typed to `TerminalViewController`. Adding WebView panels required either a union type or polymorphism.

**Decision:** Introduced `TurmPanel: AnyObject` protocol with common interface (`view`, `currentTitle`, `startIfNeeded()`, `applyBackground`, etc.). `SplitNode` uses `case leaf(any TurmPanel)`. Identity comparison uses `ObjectIdentifier` since `any TurmPanel` is not `Equatable`.

**Tradeoff:** `any TurmPanel` existentials have a small overhead vs. concrete types, but panel operations are infrequent (split/close/focus) so the overhead is negligible.

## 11. Configurable Tab Position

**Decision:** Tab bar position (`top`, `bottom`, `left`, `right`) is configurable via `[tabs] position` in config. Uses `gtk4::Notebook::set_tab_pos()`. Hot-reloads on config change.

**Rationale:** Vertical tabs (left/right) make better use of widescreen displays and are preferred by some users.
