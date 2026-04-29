use std::sync::mpsc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use webkit6::prelude::*;

use turm_core::plugin::LoadedPlugin;
use turm_core::protocol::{Request, Response};
use turm_core::theme::Theme;

use crate::panel::Panel;
use crate::socket::{EventBus, SocketCommand};

/// Backstop for `poll_reply` so a wedged dispatch path can't leave a
/// panel's `await turm.call(...)` hung forever. Without this the 5ms
/// glib re-arm runs indefinitely and the UI sticks at "loading…"
/// with no error to recover from.
///
/// MUST be longer than `service_supervisor::DEFAULT_ACTION_TIMEOUT`
/// (currently 120s — see service_supervisor.rs:54). The supervisor
/// already enforces per-action deadlines and replies with a
/// `timeout` error well before this fires, so this only triggers
/// when something deeper than the supervisor's own timer is broken.
/// 130s = 120s upper bound + 10s headroom for reply scheduling.
/// Bump in lockstep if the supervisor's action timeout grows.
const BRIDGE_REQUEST_TIMEOUT: Duration = Duration::from_secs(130);

pub struct PluginPanel {
    pub id: String,
    pub plugin_name: String,
    pub panel_name: String,
    pub title: String,
    pub container: gtk4::Box,
    pub webview: webkit6::WebView,
}

fn build_theme_css(theme: &Theme) -> String {
    format!(
        r#":root {{
    --turm-bg: {bg};
    --turm-fg: {text};
    --turm-surface0: {surface0};
    --turm-surface1: {surface1};
    --turm-surface2: {surface2};
    --turm-overlay0: {overlay0};
    --turm-text: {text};
    --turm-subtext0: {subtext0};
    --turm-subtext1: {subtext1};
    --turm-accent: {accent};
    --turm-red: {red};
}}
html, body {{
    background-color: transparent;
    color: {text};
    font-family: system-ui, -apple-system, sans-serif;
    margin: 0;
    padding: 0;
}}"#,
        bg = theme.background,
        text = theme.text,
        surface0 = theme.surface0,
        surface1 = theme.surface1,
        surface2 = theme.surface2,
        overlay0 = theme.overlay0,
        subtext0 = theme.subtext0,
        subtext1 = theme.subtext1,
        accent = theme.accent,
        red = theme.red,
    )
}

fn build_bridge_js(plugin_name: &str, panel_name: &str, panel_id: &str) -> String {
    format!(
        r#"(() => {{
    const _listeners = {{}};
    window.turm = {{
        panel: {{
            id: {id},
            name: {name},
            plugin: {plugin},
        }},
        async call(method, params = {{}}) {{
            const resp = await window.webkit.messageHandlers.turm.postMessage(
                JSON.stringify({{ method, params }})
            );
            const parsed = JSON.parse(resp);
            if (!parsed.ok) {{
                // Preserve the structured error code on the thrown
                // Error so panel-side branches (e.g. routing
                // `service_unavailable` to a transport-error view
                // vs `not_authenticated` to a setup view) can
                // discriminate. Without this, every failure
                // collapses into a single message string and
                // panels lose the ability to react meaningfully.
                const err = new Error(parsed.error?.message || "Unknown error");
                err.code = parsed.error?.code;
                throw err;
            }}
            return parsed.result;
        }},
        on(type, callback) {{
            if (!_listeners[type]) _listeners[type] = [];
            _listeners[type].push(callback);
        }},
        off(type, callback) {{
            if (!_listeners[type]) return;
            _listeners[type] = _listeners[type].filter(cb => cb !== callback);
        }},
        _handleEvent(type, data) {{
            const cbs = _listeners[type] || [];
            for (const cb of cbs) {{
                try {{ cb(data); }} catch (e) {{ console.error("turm event handler error:", e); }}
            }}
            const wildcards = _listeners["*"] || [];
            for (const cb of wildcards) {{
                try {{ cb(type, data); }} catch (e) {{ console.error("turm event handler error:", e); }}
            }}
        }},
    }};
}})()"#,
        id = serde_json::to_string(panel_id).unwrap(),
        name = serde_json::to_string(panel_name).unwrap(),
        plugin = serde_json::to_string(plugin_name).unwrap(),
    )
}

/// Create a JSC string value for replying to script messages.
fn jsc_string(ctx: &javascriptcore6::Context, s: &str) -> javascriptcore6::Value {
    javascriptcore6::Value::new_string(ctx, Some(s))
}

/// Send a Response back through the ScriptMessageReply using a JSC context.
fn reply_json(
    reply: &webkit6::ScriptMessageReply,
    ctx: &javascriptcore6::Context,
    resp: &Response,
) {
    let json = serde_json::to_string(resp).unwrap();
    reply.return_value(&jsc_string(ctx, &json));
}

impl PluginPanel {
    pub fn new(
        plugin: &LoadedPlugin,
        panel_def: &turm_core::plugin::PluginPanelDef,
        theme: &Theme,
        dispatch_tx: mpsc::Sender<SocketCommand>,
        event_bus: EventBus,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let plugin_name = plugin.manifest.plugin.name.clone();
        let panel_name = panel_def.name.clone();
        let title = panel_def.title.clone();

        let content_manager = webkit6::UserContentManager::new();

        // Inject theme CSS
        let theme_css = build_theme_css(theme);
        let style_sheet = webkit6::UserStyleSheet::new(
            &theme_css,
            webkit6::UserContentInjectedFrames::AllFrames,
            webkit6::UserStyleLevel::User,
            &[],
            &[],
        );
        content_manager.add_style_sheet(&style_sheet);

        // Inject bridge JS
        let bridge_js = build_bridge_js(&plugin_name, &panel_name, &id);
        let script = webkit6::UserScript::new(
            &bridge_js,
            webkit6::UserContentInjectedFrames::AllFrames,
            webkit6::UserScriptInjectionTime::Start,
            &[],
            &[],
        );
        content_manager.add_script(&script);

        // Dedicated web context for process isolation + sandbox paths
        let web_context = webkit6::WebContext::new();
        web_context.add_path_to_sandbox(&plugin.dir, true);
        web_context.add_path_to_sandbox("/tmp", false);

        let webview = webkit6::WebView::builder()
            .web_context(&web_context)
            .user_content_manager(&content_manager)
            .build();

        // Settings
        if let Some(settings) = webkit6::prelude::WebViewExt::settings(&webview) {
            settings.set_enable_javascript(true);
            settings.set_allow_file_access_from_file_urls(true);
            settings.set_allow_universal_access_from_file_urls(false);
            settings.set_enable_developer_extras(true);
            settings.set_hardware_acceleration_policy(webkit6::HardwareAccelerationPolicy::Always);
        }

        webview.set_hexpand(true);
        webview.set_vexpand(true);
        // Make the webview composite transparently so the window-level
        // BackgroundLayer shows through plugin panels (todo, etc.)
        // when the page itself doesn't paint a solid bg. Plugin authors
        // who want an opaque card UI add the bg themselves on inner
        // elements.
        webview.set_background_color(&gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));

        // Register JS bridge message handler with reply
        let tx = dispatch_tx;
        content_manager.register_script_message_handler_with_reply("turm", None);
        content_manager.connect_script_message_with_reply_received(
            Some("turm"),
            move |_cm, js_value: &javascriptcore6::Value, reply: &webkit6::ScriptMessageReply| {
                let ctx = js_value.context().unwrap();
                let msg_str = js_value.to_str().to_string();

                #[derive(serde::Deserialize)]
                struct BridgeMessage {
                    method: String,
                    #[serde(default)]
                    params: serde_json::Value,
                }

                let parsed: Result<BridgeMessage, _> = serde_json::from_str(&msg_str);
                match parsed {
                    Ok(msg) => {
                        let request =
                            Request::new(uuid::Uuid::new_v4().to_string(), msg.method, msg.params);
                        let (reply_tx, reply_rx) = mpsc::channel();
                        let cmd = SocketCommand {
                            request,
                            reply: reply_tx,
                        };
                        if tx.send(cmd).is_ok() {
                            let reply_clone = reply.clone();
                            let ctx_clone = ctx.clone();
                            let deadline = Instant::now() + BRIDGE_REQUEST_TIMEOUT;
                            gtk4::glib::timeout_add_local_once(
                                std::time::Duration::from_millis(1),
                                move || {
                                    poll_reply(reply_rx, reply_clone, ctx_clone, deadline);
                                },
                            );
                        } else {
                            let resp = Response::error(
                                String::new(),
                                "internal_error",
                                "Dispatch channel closed",
                            );
                            reply_json(reply, &ctx, &resp);
                        }
                    }
                    Err(e) => {
                        let resp = Response::error(
                            String::new(),
                            "parse_error",
                            &format!("Invalid bridge message: {e}"),
                        );
                        reply_json(reply, &ctx, &resp);
                    }
                }
                true
            },
        );

        // Load the HTML file
        let file_path = plugin.dir.join(&panel_def.file);
        let uri = format!("file://{}", file_path.display());

        // Diagnostic instrumentation — without these, a stuck panel
        // (the "fresh-boot shows blank panel, second turm process
        // unsticks it" symptom users have hit on cold-boot only —
        // hot reboots reproduce reliably, hot turm restarts don't)
        // leaves no log trace because WebKit's load failures and
        // WebProcess crashes go silent by default. Each handler
        // eprintlns a one-line tag with the panel's id + plugin so
        // we can correlate against the WebProcess pids in lsof /
        // journalctl when reproduction happens. Cost: three signal
        // connections per panel.
        //
        // No auto-reload here. Plugin panels carry side effects on
        // load (`terminal.exec`, action invocations from `<script>`
        // top-level), so a host-injected reload could duplicate
        // those. Idempotent retries belong in the panel's own JS —
        // see todo/panel.html's `loadGen` retry budget for the
        // canonical pattern. The host's job is just to surface the
        // failure mode loud enough that an authoring plugin knows
        // when to retry.
        let panel_label = format!("{plugin_name}/{panel_name}");
        {
            let label = panel_label.clone();
            webview.connect_load_changed(move |_, event| {
                eprintln!("[panel:{label}] load_changed: {event:?}");
            });
        }
        {
            let label = panel_label.clone();
            webview.connect_load_failed(move |_, _evt, failing_uri, err| {
                eprintln!("[panel:{label}] load_failed: uri={failing_uri} err={err}");
                false // false = don't suppress default handler
            });
        }
        {
            let label = panel_label.clone();
            webview.connect_web_process_terminated(move |_, reason| {
                eprintln!("[panel:{label}] web_process_terminated: {reason:?}");
            });
        }

        // Wayland workspace-switch freeze workaround. Symptom:
        // user opens a plugin panel → switches Hyprland workspace
        // away → comes back → panel is stuck on the last frame
        // (backend healthy, WebProcess alive, only rendering is
        // frozen). Recovers when the user opens dev-tools, clicks
        // inside, OR focuses another window and comes back. The
        // last clue is the load-bearing one: refocusing turm
        // through a focus path revives it.
        //
        // Round 1 of this fix hooked `connect_map` thinking
        // Hyprland would unmap/remap the wl_surface on workspace
        // change. It doesn't — wlroots scene-graph hides the
        // surface without unmapping, so `map` never fires for the
        // workspace toggle. Round 2 (this) hooks the toplevel
        // window's `is-active` notify, which DOES toggle on
        // workspace switch (the focused window changes when the
        // active workspace changes). When `is_active` flips back
        // to true (= turm window regained focus, whether via
        // focus-back or workspace-switch-back), nudge the JS
        // scheduler with a trivial evaluate so WebKit's compositor
        // schedules layout + paint and pushes a fresh frame.
        //
        // Same-window focus changes already self-recover without
        // code, because GTK's natural focus handling on the
        // WebView itself triggers redraws when widget focus
        // arrives. Workspace switches leave the WebView focused
        // throughout (only the toplevel's `is_active` changes), so
        // widget-level focus doesn't fire — that's why we hook the
        // toplevel.
        //
        // We connect through `connect_realize` because the widget's
        // `root()` (= toplevel window) is only valid once the
        // widget is in the window tree. Realization fires once for
        // the panel's lifetime, so this installs exactly one
        // is_active handler per panel.
        //
        // Distinct from the cold-boot prewarm (window.rs) — that
        // warms host-side daemons before any panel opens; this
        // recovers a per-panel frozen-frame after the panel is
        // already alive.
        {
            let label = panel_label.clone();
            webview.connect_realize(move |wv| {
                let Some(root) = wv.root() else { return };
                let Some(window) = root.downcast_ref::<gtk4::Window>() else {
                    return;
                };
                let wv_for_handler = wv.clone();
                let label_for_handler = label.clone();
                window.connect_is_active_notify(move |w| {
                    if w.is_active() {
                        wv_for_handler.evaluate_javascript(
                            "0",
                            None,
                            None,
                            gtk4::gio::Cancellable::NONE,
                            |_| {},
                        );
                        eprintln!("[panel:{label_for_handler}] is_active=true: nudged compositor");
                    }
                });
            });
        }

        webview.load_uri(&uri);

        // Forward events from EventBus to webview JS
        {
            let wv = webview.clone();
            let rx = event_bus.subscribe("*");

            gtk4::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                while let Some(event) = rx.try_recv() {
                    let type_escaped = serde_json::to_string(&event.kind).unwrap();
                    let data_json = serde_json::to_string(&event.payload).unwrap();
                    let js = format!(
                        "if (window.turm && window.turm._handleEvent) turm._handleEvent({type_escaped}, {data_json})"
                    );
                    wv.evaluate_javascript(&js, None, None, gtk4::gio::Cancellable::NONE, |_| {});
                }
                gtk4::glib::ControlFlow::Continue
            });
        }

        // Container (full-bleed, no toolbar)
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(&webview);

        Self {
            id,
            plugin_name,
            panel_name,
            title,
            container,
            webview,
        }
    }
}

fn poll_reply(
    rx: mpsc::Receiver<Response>,
    reply: webkit6::ScriptMessageReply,
    ctx: javascriptcore6::Context,
    deadline: Instant,
) {
    match rx.try_recv() {
        Ok(response) => {
            reply_json(&reply, &ctx, &response);
        }
        Err(mpsc::TryRecvError::Empty) => {
            if Instant::now() >= deadline {
                let resp = Response::error(
                    String::new(),
                    "bridge_timeout",
                    &format!(
                        "no reply within {}s — dispatcher stalled or action wedged",
                        BRIDGE_REQUEST_TIMEOUT.as_secs()
                    ),
                );
                reply_json(&reply, &ctx, &resp);
                return;
            }
            gtk4::glib::timeout_add_local_once(Duration::from_millis(5), move || {
                poll_reply(rx, reply, ctx, deadline);
            });
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            let resp = Response::error(String::new(), "internal_error", "Reply channel closed");
            reply_json(&reply, &ctx, &resp);
        }
    }
}

impl Panel for PluginPanel {
    fn widget(&self) -> &gtk4::Widget {
        self.container.upcast_ref()
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn panel_type(&self) -> &str {
        "plugin"
    }

    fn grab_focus(&self) {
        self.webview.grab_focus();
    }

    fn id(&self) -> &str {
        &self.id
    }
}
