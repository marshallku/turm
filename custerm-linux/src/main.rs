mod app;
mod dbus;
mod panel;
mod search;
mod socket;
mod split;
mod tabs;
mod terminal;
mod webview;
mod window;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("custerm {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.iter().any(|a| a == "--init-config") {
        match custerm_core::config::CustermConfig::write_default() {
            Ok(path) => {
                println!("Config written to: {}", path.display());
                return;
            }
            Err(e) => {
                eprintln!("Failed to write config: {e}");
                std::process::exit(1);
            }
        }
    }

    if args.iter().any(|a| a == "--config-path") {
        println!(
            "{}",
            custerm_core::config::CustermConfig::config_path().display()
        );
        return;
    }

    app::run();
}
