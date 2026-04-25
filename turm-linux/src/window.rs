use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, gio, glib};
use serde_json::json;

use turm_core::action_registry::{ActionRegistry, internal_error};
use turm_core::config::TurmConfig;
use turm_core::context::ContextService;
use turm_core::event_bus::EventReceiver;

use crate::panel::Panel;
use crate::socket;
use crate::statusbar::StatusBar;
use crate::tabs::TabManager;

pub struct TurmWindow {
    pub window: ApplicationWindow,
    pub tab_manager: Rc<TabManager>,
    #[allow(dead_code)]
    statusbar: Rc<StatusBar>,
}

impl TurmWindow {
    pub fn new(app: &Application, config: &TurmConfig) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("turm")
            .default_width(1200)
            .default_height(800)
            .build();

        let theme = turm_core::theme::Theme::by_name(&config.theme.name).unwrap_or_default();
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(&format!(
            "window {{ background-color: {}; }}",
            theme.background
        ));
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let event_bus = socket::new_event_bus();

        // Context Service: live snapshot of "what the user is currently doing."
        // Pumped from the GTK timer below; exposed via the `context.snapshot` action.
        //
        // Exact-match subscriptions per consumed kind (NOT `"*"` and NOT
        // `panel.*`). High-frequency events (`terminal.output`) and any
        // potentially-spammy sibling under the `panel.*` namespace
        // (`panel.title_changed`) must never share a bounded buffer with the
        // state-changing events Context actually consumes, or drop-newest
        // semantics could silently evict them and leave context stale until a
        // compensating event happens.
        let context = Arc::new(ContextService::new());
        let context_focused_rx = event_bus.subscribe("panel.focused");
        let context_exited_rx = event_bus.subscribe("panel.exited");
        let context_cwd_rx = event_bus.subscribe("terminal.cwd_changed");

        // Action Registry: shared across socket + plugin dispatch paths.
        // Migrating commands one at a time from the match arm in socket::dispatch.
        let actions = Arc::new(ActionRegistry::new());
        actions.register("system.ping", |_| Ok(json!({ "status": "ok" })));
        {
            let ctx = context.clone();
            actions.register("context.snapshot", move |_| {
                serde_json::to_value(ctx.snapshot())
                    .map_err(|e| internal_error(format!("snapshot serialization failed: {e}")))
            });
        }

        // Plugin discovery
        let plugins = turm_core::plugin::discover_plugins();
        for p in &plugins {
            eprintln!(
                "[turm] plugin loaded: {} v{}",
                p.manifest.plugin.name, p.manifest.plugin.version
            );
        }

        // Socket server (per-instance, so multiple turm windows don't collide)
        let socket_path = format!("/tmp/turm-{}.sock", std::process::id());
        let socket_rx = socket::start_server(&socket_path, event_bus.clone());

        // Create a dispatch sender for the plugin JS bridge to reuse
        let (dispatch_tx, plugin_dispatch_rx) = std::sync::mpsc::channel();

        let tab_manager = TabManager::new(
            config,
            &window,
            event_bus.clone(),
            plugins.clone(),
            dispatch_tx,
        );

        // Status bar
        let statusbar = Rc::new(StatusBar::new(config, &plugins));

        // Layout: vertical box with notebook + statusbar
        let layout = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        if config.statusbar.position == "top" {
            layout.append(&statusbar.container);
            layout.append(&tab_manager.notebook);
        } else {
            layout.append(&tab_manager.notebook);
            layout.append(&statusbar.container);
        }
        window.set_child(Some(&layout));

        // Config hot-reload
        watch_config(&tab_manager, &statusbar, &plugins);

        let mgr = tab_manager.clone();
        let win = window.clone();
        let sp = socket_path.clone();
        let sb = statusbar.clone();
        let act = actions.clone();
        let ctx_pump = context.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            // Drain bus events into ContextService BETWEEN every dispatched
            // command (not just at the top of the tick). A dispatched command
            // can publish events — e.g. `tab.new` → `panel.focused` — and the
            // very next command in the same batch (e.g. `context.snapshot`)
            // must see those events applied. A single drain at the top would
            // leave such a same-tick reader stale for one full timer period.
            // Order across the three receivers is not significant: focused,
            // exited, and cwd_changed for different panels are commutative
            // for context's state model.
            drain_context(
                &ctx_pump,
                &context_focused_rx,
                &context_exited_rx,
                &context_cwd_rx,
            );
            while let Ok(cmd) = socket_rx.try_recv() {
                socket::dispatch(cmd, &mgr, &win, &sp, &sb, &act);
                drain_context(
                    &ctx_pump,
                    &context_focused_rx,
                    &context_exited_rx,
                    &context_cwd_rx,
                );
            }
            while let Ok(cmd) = plugin_dispatch_rx.try_recv() {
                socket::dispatch(cmd, &mgr, &win, &sp, &sb, &act);
                drain_context(
                    &ctx_pump,
                    &context_focused_rx,
                    &context_exited_rx,
                    &context_cwd_rx,
                );
            }
            glib::ControlFlow::Continue
        });

        // Cleanup socket on shutdown
        let socket_path_cleanup = socket_path.clone();
        window.connect_destroy(move |_| {
            socket::cleanup(&socket_path_cleanup);
        });

        Self {
            window,
            tab_manager,
            statusbar,
        }
    }

    pub fn present(&self) {
        self.window.present();
        // Focus the terminal after the window is mapped
        let mgr = self.tab_manager.clone();
        glib::idle_add_local_once(move || {
            if let Some(panel) = mgr.active_panel() {
                panel.grab_focus();
            }
        });
    }
}

fn watch_config(
    tab_manager: &Rc<TabManager>,
    statusbar: &Rc<StatusBar>,
    plugins: &[turm_core::plugin::LoadedPlugin],
) {
    let config_path = TurmConfig::config_path();
    let file = gio::File::for_path(&config_path);

    let monitor = match file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[turm] failed to watch config: {}", e);
            return;
        }
    };

    let mgr = tab_manager.clone();
    let sb = statusbar.clone();
    let pl = plugins.to_vec();
    monitor.connect_changed(move |_, _, _, event| {
        if !matches!(
            event,
            gio::FileMonitorEvent::Changed | gio::FileMonitorEvent::Created
        ) {
            return;
        }

        let config = match TurmConfig::load() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[turm] config reload error: {}", e);
                return;
            }
        };

        eprintln!("[turm] config reloaded");
        mgr.update_config(&config);
        sb.reload(&config, &pl);
    });

    std::mem::forget(monitor);
}

fn drain_context(
    ctx: &ContextService,
    rx_focused: &EventReceiver,
    rx_exited: &EventReceiver,
    rx_cwd: &EventReceiver,
) {
    while let Some(event) = rx_focused.try_recv() {
        ctx.apply_event(&event);
    }
    while let Some(event) = rx_exited.try_recv() {
        ctx.apply_event(&event);
    }
    while let Some(event) = rx_cwd.try_recv() {
        ctx.apply_event(&event);
    }
}
