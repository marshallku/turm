use std::sync::mpsc;

use gtk4::prelude::*;
use webkit6::prelude::*;

use turm_core::config::TurmConfig;
use turm_core::plugin::LoadedPlugin;
use turm_core::protocol::{Request, Response};
use turm_core::theme::Theme;

use crate::socket::{EventBus, SocketCommand};

pub struct StatusBar {
    pub container: gtk4::Box,
    webview: webkit6::WebView,
}

/// Build the shell HTML that hosts all modules.
/// Each module's HTML is injected into left/center/right containers.
fn build_bar_html(plugins: &[LoadedPlugin], theme: &Theme, height: u32) -> String {
    // Collect modules from all plugins, sorted by (position, order)
    let mut left = Vec::new();
    let mut center = Vec::new();
    let mut right = Vec::new();

    for plugin in plugins {
        for module in &plugin.manifest.modules {
            let file_path = plugin.dir.join(&module.file);
            let content = match std::fs::read_to_string(&file_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "[turm] failed to read module {}/{}: {e}",
                        plugin.manifest.plugin.name, module.name
                    );
                    continue;
                }
            };

            let entry = (module.order, module.name.clone(), content);
            match module.position.as_str() {
                "left" => left.push(entry),
                "center" => center.push(entry),
                _ => right.push(entry),
            }
        }
    }

    left.sort_by_key(|(o, _, _)| *o);
    center.sort_by_key(|(o, _, _)| *o);
    right.sort_by_key(|(o, _, _)| *o);

    let render_section = |modules: &[(i32, String, String)]| -> String {
        modules
            .iter()
            .map(|(_, name, html)| {
                format!(r#"<div class="turm-module" data-module="{name}">{html}</div>"#)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<style>
:root {{
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
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
html, body {{
    height: {height}px;
    overflow: hidden;
    background: {surface0};
    color: {text};
    font-family: system-ui, -apple-system, sans-serif;
    font-size: 12px;
}}
body {{
    display: flex;
    align-items: center;
    border-top: 1px solid {overlay0};
}}
#left, #center, #right {{
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 10px;
}}
#left {{ justify-content: flex-start; }}
#center {{ flex: 1; justify-content: center; }}
#right {{ justify-content: flex-end; }}
.turm-module {{
    display: inline-flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
}}
</style>
</head>
<body>
<div id="left">{left}</div>
<div id="center">{center}</div>
<div id="right">{right}</div>
</body>
</html>"#,
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
        height = height,
        left = render_section(&left),
        center = render_section(&center),
        right = render_section(&right),
    )
}

fn build_bridge_js() -> String {
    r#"(() => {
    const _listeners = {};
    window.turm = {
        panel: { id: "__statusbar__", name: "statusbar", plugin: "turm" },
        async call(method, params = {}) {
            const resp = await window.webkit.messageHandlers.turm.postMessage(
                JSON.stringify({ method, params })
            );
            const parsed = JSON.parse(resp);
            if (!parsed.ok) throw new Error(parsed.error?.message || "Unknown error");
            return parsed.result;
        },
        on(type, callback) {
            if (!_listeners[type]) _listeners[type] = [];
            _listeners[type].push(callback);
        },
        off(type, callback) {
            if (!_listeners[type]) return;
            _listeners[type] = _listeners[type].filter(cb => cb !== callback);
        },
        _handleEvent(type, data) {
            const cbs = _listeners[type] || [];
            for (const cb of cbs) {
                try { cb(data); } catch (e) { console.error("turm event handler error:", e); }
            }
            const wildcards = _listeners["*"] || [];
            for (const cb of wildcards) {
                try { cb(type, data); } catch (e) { console.error("turm event handler error:", e); }
            }
        },
    };
})()"#
        .to_string()
}

/// JSC string value helper
fn jsc_string(ctx: &javascriptcore6::Context, s: &str) -> javascriptcore6::Value {
    javascriptcore6::Value::new_string(ctx, Some(s))
}

fn reply_json(
    reply: &webkit6::ScriptMessageReply,
    ctx: &javascriptcore6::Context,
    resp: &Response,
) {
    let json = serde_json::to_string(resp).unwrap();
    reply.return_value(&jsc_string(ctx, &json));
}

impl StatusBar {
    pub fn new(
        config: &TurmConfig,
        plugins: &[LoadedPlugin],
        dispatch_tx: mpsc::Sender<SocketCommand>,
        event_bus: EventBus,
    ) -> Self {
        let theme = Theme::by_name(&config.theme.name).unwrap_or_default();
        let height = config.statusbar.height;

        // Check if any plugins have modules
        let has_modules = plugins.iter().any(|p| !p.manifest.modules.is_empty());
        if !has_modules {
            eprintln!("[turm] statusbar: no modules found, bar will be empty");
        }

        let content_manager = webkit6::UserContentManager::new();

        // Inject bridge JS
        let bridge_js = build_bridge_js();
        let script = webkit6::UserScript::new(
            &bridge_js,
            webkit6::UserContentInjectedFrames::AllFrames,
            webkit6::UserScriptInjectionTime::Start,
            &[],
            &[],
        );
        content_manager.add_script(&script);

        let webview = webkit6::WebView::builder()
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
        webview.set_vexpand(false);
        webview.set_size_request(-1, height as i32);

        // Register JS bridge message handler
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
                            gtk4::glib::timeout_add_local_once(
                                std::time::Duration::from_millis(1),
                                move || {
                                    poll_reply(reply_rx, reply_clone, ctx_clone);
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

        // Build and load the bar HTML
        let html = build_bar_html(plugins, &theme, height);
        webview.load_html(&html, None);

        // Forward events to webview
        {
            let wv = webview.clone();
            let (etx, erx) = mpsc::channel::<String>();
            event_bus.lock().unwrap().push(etx);

            gtk4::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                while let Ok(event_json) = erx.try_recv() {
                    if let Ok(event) =
                        serde_json::from_str::<turm_core::protocol::Event>(&event_json)
                    {
                        let type_escaped = serde_json::to_string(&event.event_type).unwrap();
                        let data_json = serde_json::to_string(&event.data).unwrap();
                        let js = format!(
                            "if (window.turm && window.turm._handleEvent) turm._handleEvent({type_escaped}, {data_json})"
                        );
                        wv.evaluate_javascript(
                            &js,
                            None,
                            None,
                            gtk4::gio::Cancellable::NONE,
                            |_| {},
                        );
                    }
                }
                gtk4::glib::ControlFlow::Continue
            });
        }

        // Container
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(false);
        container.append(&webview);

        if !config.statusbar.enabled {
            container.set_visible(false);
        }

        Self { container, webview }
    }

    pub fn set_visible(&self, visible: bool) {
        self.container.set_visible(visible);
    }

    pub fn is_visible(&self) -> bool {
        self.container.is_visible()
    }

    pub fn toggle(&self) -> bool {
        let new_visible = !self.is_visible();
        self.set_visible(new_visible);
        new_visible
    }

    /// Reload bar content (e.g., after plugin changes)
    pub fn reload(&self, config: &TurmConfig, plugins: &[LoadedPlugin]) {
        let theme = Theme::by_name(&config.theme.name).unwrap_or_default();
        let html = build_bar_html(plugins, &theme, config.statusbar.height);
        self.webview.load_html(&html, None);
    }
}

fn poll_reply(
    rx: mpsc::Receiver<Response>,
    reply: webkit6::ScriptMessageReply,
    ctx: javascriptcore6::Context,
) {
    match rx.try_recv() {
        Ok(response) => {
            reply_json(&reply, &ctx, &response);
        }
        Err(mpsc::TryRecvError::Empty) => {
            gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(5), move || {
                poll_reply(rx, reply, ctx);
            });
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            let resp = Response::error(String::new(), "internal_error", "Reply channel closed");
            reply_json(&reply, &ctx, &resp);
        }
    }
}
