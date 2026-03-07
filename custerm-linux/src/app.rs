use gtk4::prelude::*;
use gtk4::{Application, gio};

use crate::window::CustermWindow;

const APP_ID: &str = "com.marshall.custerm";

pub fn run() {
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_startup(|_| {
        if let Some(settings) = gtk4::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(true);
        }
    });

    app.connect_activate(|app| {
        let config = custerm_core::config::CustermConfig::load().unwrap_or_default();
        let window = CustermWindow::new(app, &config);
        window.present();
    });

    app.run();
}
