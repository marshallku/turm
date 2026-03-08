use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, gio, glib};

use turm_core::config::TurmConfig;

use crate::dbus::{self, DbusCommand};
use crate::panel::Panel;
use crate::socket;
use crate::tabs::TabManager;

pub struct TurmWindow {
    pub window: ApplicationWindow,
    pub tab_manager: Rc<TabManager>,
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
        let tab_manager = TabManager::new(config, &window, event_bus.clone());

        window.set_child(Some(&tab_manager.notebook));

        // Config hot-reload
        watch_config(&tab_manager);

        // D-Bus: apply to active terminal panel (only if it's a terminal)
        let rx = dbus::register();
        let mgr = tab_manager.clone();
        glib::timeout_add_local(Duration::from_millis(150), move || {
            while let Ok(cmd) = rx.try_recv() {
                if let Some(panel) = mgr.active_panel()
                    && let Some(term) = panel.as_terminal()
                {
                    match cmd {
                        DbusCommand::SetBackground(path) => {
                            term.set_background(std::path::Path::new(&path));
                        }
                        DbusCommand::ClearBackground => {
                            term.clear_background();
                        }
                        DbusCommand::SetTint(opacity) => {
                            term.set_tint(opacity);
                        }
                    }
                }
            }
            glib::ControlFlow::Continue
        });

        // Socket server (per-instance, so multiple turm windows don't collide)
        let socket_path = format!("/tmp/turm-{}.sock", std::process::id());
        let socket_rx = socket::start_server(&socket_path, event_bus);
        let mgr = tab_manager.clone();
        let win = window.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            while let Ok(cmd) = socket_rx.try_recv() {
                socket::dispatch(cmd, &mgr, &win);
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

fn watch_config(tab_manager: &Rc<TabManager>) {
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
    });

    std::mem::forget(monitor);
}
