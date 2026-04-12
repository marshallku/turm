use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, gio, glib};

use turm_core::config::TurmConfig;

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
        glib::timeout_add_local(Duration::from_millis(50), move || {
            // Process commands from socket server
            while let Ok(cmd) = socket_rx.try_recv() {
                socket::dispatch(cmd, &mgr, &win, &sp, &sb);
            }
            // Process commands from plugin JS bridges
            while let Ok(cmd) = plugin_dispatch_rx.try_recv() {
                socket::dispatch(cmd, &mgr, &win, &sp, &sb);
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
