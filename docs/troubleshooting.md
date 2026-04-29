# Troubleshooting

## Build Issues

### Missing vte4 system library

```
error: could not find system library 'vte-2.91-gtk4'
```

**Fix:** `sudo pacman -S vte4`

### Missing gtk4 system library

**Fix:** `sudo pacman -S gtk4`

### `load_from_string` not found on CssProvider

The method is gated behind a feature flag.
**Fix:** Add `features = ["gnome_46"]` to gtk4 dependency in Cargo.toml.

### Cargo binary name collision

```
warning: output filename collision at target/debug/turm
```

turm-linux and turm-cli both output `turm`.
**Fix:** CLI binary renamed to `turmctl` in turm-cli/Cargo.toml.

## Runtime Issues

### Wayland protocol error (Error 71)

```
Gdk-Message: Error 71 (Protocol error)
```

**Fix:** Set `GDK_BACKEND=x11` in environment or in main.rs.

### GBM buffer error

```
Failed to create GBM buffer of size 841x1352: Invalid argument
```

**Fix:** Set `WEBKIT_DISABLE_DMABUF_RENDERER=1` (only relevant if using WebKit components).

### Terminal shows in light mode

**Cause:** Transparent VTE background with no image loaded shows the system theme underneath.
**Fix:**

1. Force dark theme: `settings.set_gtk_application_prefer_dark_theme(true)` in `app.rs`
2. Window CSS `window { background-color: <theme.background> }` provides the solid fallback color now that VTE is permanently transparent (no more conditional opaque bg).

### Background images not showing (solid color only)

Multiple possible causes:

1. **Config `directory` is commented out**: Check `~/.config/turm/config.toml`. The `directory` field must be uncommented. A `#` before the key comments it out.

2. **Surface is opaque**: the window-level `BackgroundLayer` paints behind everything, so any opaque widget above it hides the image. Required transparent surfaces: VTE (`set_clear_background(false)` + `RGBA(0,0,0,0)`), WebKit (`webview.set_background_color(RGBA(0,0,0,0))`), notebook header / statusbar / `html, body` in plugin CSS — all transparent. If you add a new chrome widget and the image disappears under it, that widget needs the same treatment.

3. **Image loading fails silently**: The original `GtkPicture::set_file()` loads asynchronously and fails silently. Fixed by using `gdk::Texture::from_file()` for synchronous loading with error reporting.

4. **Tint too opaque**: Tint at 0.9 makes images nearly invisible (90% opaque dark overlay). Lower to 0.85 or less.

5. **GTK single-instance**: If an old turm is running, new launches activate the old instance and exit immediately (exit code 0, no output). Kill all instances first: `killall turm`.

### App exits immediately with no error

**Cause:** GTK single-instance behavior. Another turm instance already owns the GTK app ID `com.marshall.turm`.
**Fix:** `killall turm` then relaunch.

### env_logger output not visible

**Cause:** GTK may capture/redirect stderr. `RUST_LOG=info` has no visible effect.
**Fix:** Use `eprintln!("[turm] ...")` instead of `log::info!()` for debug output.

### Terminal shows only one line (collapsed height)

**Cause:** `GtkOverlay` sizes based on its child widget. The window-level root overlay's child is the (hideable) `bg_picture` from `BackgroundLayer`, so when no image is set the base child has zero natural size and the overlay collapses unless an overlay is marked as size-driver.
**Fix:** Call `root_overlay.set_measure_overlay(&layout, true)` so the actual UI layout drives the overlay's measurement regardless of bg image state. The `TerminalPanel`'s own (search-bar) overlay is already measured by its terminal child.

### WebKit web process crashes on many sites

```
GStreamer element autoaudiosink not found. Please install it
GLib-GObject-CRITICAL: invalid (NULL) pointer instance
WebProcess CRASHED
```

**Cause:** Missing GStreamer plugins. WebKitGTK requires GStreamer for media handling, and crashes when the plugins are absent — even on pages that don't play media.
**Fix:** `sudo pacman -S gst-plugins-good gst-plugins-bad`

### D-Bus: `register_object` API mismatch

**Cause:** gio 0.20 uses builder pattern, not positional args.
**Fix:** Use `connection.register_object(path, &interface_info).method_call(closure).build()`.

### Plugin/webview panel frozen on last frame after Hyprland workspace switch

**Symptom:** Plugin panel (or any `webkit6::WebView`) renders fine on first show. User switches to a different Hyprland workspace, then comes back. Panel is stuck on the last frame — appears alive but doesn't repaint, doesn't respond to bus events visually. Right-clicking → "Inspect Element" instantly revives the panel; OR focusing another window and coming back also revives it.

**Cause:** When Hyprland (or any wlroots-based compositor with scene-graph workspaces) switches workspace, the wl_surface is NOT unmapped — it's just hidden in the scene graph. So GTK4's `map` signal does NOT fire on workspace toggle. But WebKit's compositor still gets confused and stops pushing frames; opening dev tools revives it (dev-tools attach schedules a JS task → layout → paint), and so does focusing another window and coming back (the toplevel `is_active` toggles, GTK natural focus handling triggers a redraw).

The load-bearing distinction: same-window focus changes self-recover (because widget-level focus moves in/out of the WebView), but pure workspace switch keeps the WebView focused throughout — only the toplevel's `is_active` changes. So we hook at the toplevel level.

**Fix (round 2 — round 1's `connect_map` did not work because `map` doesn't fire on Hyprland workspace toggle):** Hook the toplevel window's `is-active` notify. When `is_active` flips back to true (= turm window regained focus, whether via focus-back or workspace-switch-back), run a trivial `evaluate_javascript("0", …)` on the WebView so the JS scheduler kicks WebKit into pushing a fresh frame. Connect through `connect_realize` because the widget's `root()` (= toplevel window) is only valid once the widget is in the window tree.

```rust
webview.connect_realize(|wv| {
    let Some(root) = wv.root() else { return };
    let Some(window) = root.downcast_ref::<gtk4::Window>() else { return };
    let wv2 = wv.clone();
    window.connect_is_active_notify(move |w| {
        if w.is_active() {
            wv2.evaluate_javascript("0", None, None,
                gtk4::gio::Cancellable::NONE, |_| {});
        }
    });
});
```

Applied at `turm-linux/src/plugin_panel.rs` (per-panel) and `turm-linux/src/webview.rs` (generic webview panel). Distinct from cold-boot blank panel (different mechanism — see commit `bb9c1f1` prewarm).

---

## macOS App Issues

### SwiftTerm: `processTerminated` never called after shell exits

**Cause:** SwiftTerm's `LocalProcess.childProcessRead` detects PTY EOF and calls `childStopped()`, which cancels the internal `childMonitor` DispatchSource before it can fire. The `processTerminated` call in the EOF handler is commented out in SwiftTerm source.

**Fix:** Install a separate `DispatchSource.makeProcessSource` after `startProcess()` returns (in `TurmTerminalView.installExitMonitor()`). This source is not affected by `childStopped()` and fires independently when the process exits.

```swift
func installExitMonitor() {
    let pid = process.shellPid
    guard pid > 0 else { return }
    let src = DispatchSource.makeProcessSource(identifier: pid, eventMask: .exit, queue: .main)
    src.setEventHandler { [weak self, weak src] in
        src?.cancel()
        guard let self else { return }
        processDelegate?.processTerminated(source: self, exitCode: nil)
    }
    exitMonitor = src
    src.activate()
}
```

### macOS split panes: new pane gets wrong initial size

**Cause 1 (`layout()` approach):** NSSplitView calls `resizeSubviews` (which sets subview frames) before calling `layout()`. By the time `layout()` fires, the wrong frames are already committed. Calling `setPosition` in `layout()` fires too late — if the terminal view already has a large frame from before the rebuild, NSSplitView uses that as the basis for proportional sizing.

**Cause 2 (`asyncAfter` approach):** The 50ms delay is unreliable — layout may not have resolved yet, or a subsequent split may have started before the timer fires, applying stale positions.

**Fix:** Use `NSSplitViewDelegate.splitView(_:resizeSubviewsWithOldSize:)`. This delegate method is called by NSSplitView at the exact moment it needs to determine subview frames. Set frames directly here and set `initialSizeSet = true` after the first call to fall back to `adjustSubviews()` for subsequent resizes (preserving user drag behaviour).

### macOS: `becomeFirstResponder` cannot be overridden in SwiftTerm subclass

**Cause:** `MacTerminalView.becomeFirstResponder` is declared `public` but not `open`, so it cannot be overridden by code outside the SwiftTerm module.

**Fix:** Use `NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown)` in `PaneManager` to detect which pane was clicked and update `activePane` accordingly.

### macOS: `terminal.output` event not implementable

**Cause:** SwiftTerm's `feed(byteArray:)` is declared in an extension of `TerminalView` (not `open`), so it cannot be overridden by subclasses outside the module. There is no other public hook for intercepting raw PTY output bytes.

**Status:** Not implemented. Shell integration signals (`terminal.shell_precmd` / `terminal.shell_preexec`) are sent via socket commands from the shell script directly instead of OSC 133 parsing.

### macOS: Nerd Font icons show as boxes or render broken

**Cause 1 — Font not found by family name:** `NSFont(name:size:)` only accepts PostScript names and full names (e.g. `JetBrains Mono Regular`), not bare family names like `JetBrainsMono Nerd Font Mono`. When the lookup fails, the terminal falls back to the system monospace font which has no Nerd Font PUA glyphs.

**Fix:** Font resolution now uses a multi-step strategy: PostScript name → `NSFontManager` exact family lookup → case-insensitive family lookup → `NSFontDescriptor` → system fallback. Both PostScript names and family names now work reliably.

**Cause 2 — Using non-Mono Nerd Font variant:** Standard Nerd Font variants (e.g. `JetBrainsMono Nerd Font`) render icons as 2-column wide glyphs. SwiftTerm's Unicode width table does not include PUA codepoints (U+E000–U+F8FF), so it treats them as 1-column, causing icons to overflow into the adjacent cell.

**Fix:** Use the **Mono** variant of your Nerd Font (e.g. `JetBrainsMono Nerd Font Mono`). Mono variants explicitly set all icons to 1-column width.

```toml
[terminal]
font_family = "JetBrainsMono Nerd Font Mono"
```

### macOS: Background `opacity` config change not reflected at runtime

**Cause:** `Config.swift` only parsed `path` and `tint` from the `[background]` section. The `opacity` field was silently ignored, and the `applyBackground` signature only accepted `path` and `tint`. Hot-reload therefore never changed the image layer's alpha.

**Fix:** Added `backgroundOpacity: Double` to `TurmConfig`, parse `("background", "opacity")` in `Config.parse`, and propagated an `opacity` parameter through the full call chain: `TurmPanel.applyBackground(path:tint:opacity:)` → `TerminalViewController` (sets `backgroundView?.alphaValue`) → `WebViewController` (no-op) → `PaneManager` → `TabViewController` (stores `currentBackgroundOpacity`) → `AppDelegate` initial apply and `background.set` socket command.

Also added `("background", "image")` as an alias for `("background", "path")` to match the documented config key.

### macOS: OSC 7 CWD URI includes hostname

**Cause:** OSC 7 delivers a `file://hostname/path` URI (e.g. `file://Marshalls-MacBook-Pro.local/Users/marshallku`). Simply stripping `file://` leaves the hostname in the path.

**Fix:** Use `URL(string: directory).path` to correctly extract only the POSIX path component, discarding the scheme and hostname.
