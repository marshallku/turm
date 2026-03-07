use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, gio, glib};

use custerm_core::config::CustermConfig;

use crate::dbus::{self, DbusCommand};
use crate::socket;
use crate::tabs::TabManager;

pub struct CustermWindow {
    pub window: ApplicationWindow,
}

impl CustermWindow {
    pub fn new(app: &Application, config: &CustermConfig) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("custerm")
            .default_width(1200)
            .default_height(800)
            .build();

        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string("window { background-color: #1e1e2e; }");
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

        // D-Bus: apply to active terminal panel
        let rx = dbus::register();
        let mgr = tab_manager.clone();
        glib::timeout_add_local(Duration::from_millis(150), move || {
            while let Ok(cmd) = rx.try_recv() {
                if let Some(panel) = mgr.active_panel() {
                    match cmd {
                        DbusCommand::SetBackground(path) => {
                            panel.set_background(std::path::Path::new(&path));
                        }
                        DbusCommand::ClearBackground => {
                            panel.clear_background();
                        }
                        DbusCommand::SetTint(opacity) => {
                            panel.set_tint(opacity);
                        }
                    }
                }
            }
            glib::ControlFlow::Continue
        });

        // Socket server
        let socket_path = "/tmp/custerm.sock".to_string();
        let socket_rx = socket::start_server(&socket_path, event_bus);
        let mgr = tab_manager.clone();
        let win = window.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            while let Ok(cmd) = socket_rx.try_recv() {
                let response = socket::dispatch(&cmd.request, &mgr, &win);
                let _ = cmd.reply.send(response);
            }
            glib::ControlFlow::Continue
        });

        // Cleanup socket on shutdown
        let socket_path_cleanup = socket_path.clone();
        window.connect_destroy(move |_| {
            socket::cleanup(&socket_path_cleanup);
        });

        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}

fn watch_config(tab_manager: &Rc<TabManager>) {    let config_path = CustermConfig::config_path();
    let file = gio::File::for_path(&config_path);

    let monitor = match file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[custerm] failed to watch config: {}", e);
            return;
        }
    };

    let mgr = tab_manager.clone();
    monitor.connect_changed(move |_, _, _, event| {
        if !matches!(event, gio::FileMonitorEvent::Changed | gio::FileMonitorEvent::Created) {
            return;
        }

        let config = match CustermConfig::load() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[custerm] config reload error: {}", e);
                return;
            }
        };

        eprintln!("[custerm] config reloaded");
        mgr.update_config(&config);
    });

    std::mem::forget(monitor);
}
