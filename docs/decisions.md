# Technical Decisions

## 1. Tauri v2 Abandoned → Native Platform UIs

**Problem:** Tauri IPC introduced noticeable input latency in the terminal. Every keypress went through JS → Tauri invoke → Rust → PTY, and output went PTY → Rust → Tauri event → JS → xterm.js. The round-trip was perceptible.

**Decision:** Switched to platform-native UIs with a shared Rust core:

- Linux: GTK4 + VTE4 (VTE handles PTY internally, zero IPC overhead)
- macOS: Swift/AppKit (SwiftTerm or Ghostty embedding, TBD)

**Tradeoff:** More code per platform, but terminal responsiveness is non-negotiable.

## 2. VTE Handles PTY on Linux

**Rationale:** VTE has its own optimized PTY management. Using `portable-pty` alongside VTE would mean double PTY handling. Let VTE do what it does best.

**Consequence:** `turm-core/pty.rs` is not used by turm-linux. It exists for macOS and potential future socket server needs.

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

## 6. Catppuccin Mocha Hardcoded

**Current state:** Theme colors are hardcoded in `terminal.rs`. The config `[theme] name = "catppuccin-mocha"` exists but theme switching is not yet implemented.

**Future:** Parse theme files or embed multiple palettes.

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

## 11. Configurable Tab Position

**Decision:** Tab bar position (`top`, `bottom`, `left`, `right`) is configurable via `[tabs] position` in config. Uses `gtk4::Notebook::set_tab_pos()`. Hot-reloads on config change.

**Rationale:** Vertical tabs (left/right) make better use of widescreen displays and are preferred by some users.
