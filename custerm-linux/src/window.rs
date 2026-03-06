use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow};
use gtk4::glib;

use custerm_core::background::BackgroundManager;
use custerm_core::config::CustermConfig;

use crate::dbus::{self, DbusCommand};
use crate::terminal::TerminalTab;

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

        let terminal = TerminalTab::new(config);
        window.set_child(Some(terminal.widget()));

        // Apply CSS
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string("window { background-color: #1e1e2e; }");
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Background manager
        let bg_dir = config.background.directory.as_deref();
        let mut bg_manager = BackgroundManager::new(bg_dir);
        let _ = bg_manager.load_cache();

        // Apply initial background
        if let Some(path) = bg_manager.next().map(|p| p.to_path_buf()) {
            terminal.set_background(&path);
        }

        let bg_manager = Arc::new(Mutex::new(bg_manager));

        // Register D-Bus and poll for commands on main thread
        let rx = dbus::register(bg_manager.clone());
        glib::timeout_add_local(Duration::from_millis(50), move || {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    DbusCommand::SetBackground(path) => {
                        terminal.set_background(std::path::Path::new(&path));
                    }
                    DbusCommand::ClearBackground => {
                        terminal.clear_background();
                    }
                    DbusCommand::SetTint(opacity) => {
                        terminal.set_tint(opacity);
                    }
                }
            }
            glib::ControlFlow::Continue
        });

        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}
