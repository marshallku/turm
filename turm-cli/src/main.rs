mod client;
mod commands;
mod plugin_cmds;
mod update;

use clap::Parser;
use commands::{Cli, Command, EventCommand, UpdateCommand};

fn main() {
    let cli = Cli::parse();

    // Handle update commands locally (no socket needed)
    if let Command::Update(cmd) = &cli.command {
        match cmd {
            UpdateCommand::Check => update::check_update(),
            UpdateCommand::Apply { version } => update::apply_update(version.clone()),
        }
        return;
    }

    let socket_path = cli.socket.clone().unwrap_or_else(|| {
        std::env::var("TURM_SOCKET")
            .ok()
            .filter(|p| std::os::unix::net::UnixStream::connect(p).is_ok())
            .unwrap_or_else(|| discover_socket().unwrap_or_else(|| "/tmp/turm.sock".to_string()))
    });

    // Event subscribe is a long-lived streaming connection
    if matches!(&cli.command, Command::Event(EventCommand::Subscribe)) {
        match client::subscribe(&socket_path) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Failed to subscribe: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // Phase 19.1: per-plugin ergonomic wrappers own their dispatch
    // (preflight id resolution + custom human renderer), bypassing
    // the generic `cli.method() / cli.params()` path.
    if let Command::Todo(cmd) = &cli.command {
        std::process::exit(plugin_cmds::todo::dispatch(cmd, &socket_path, cli.json));
    }

    let result = client::send_command(&socket_path, &cli.method(), cli.params());

    match result {
        Ok(response) => {
            if response.ok {
                if let Some(result) = response.result {
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&result).unwrap());
                    } else {
                        print_result(&result);
                    }
                }
            } else if let Some(err) = response.error {
                eprintln!("Error [{}]: {}", err.code, err.message);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Failed to connect: {e}");
            std::process::exit(1);
        }
    }
}

fn discover_socket() -> Option<String> {
    let mut sockets: Vec<_> = std::fs::read_dir("/tmp")
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("turm-") && name.ends_with(".sock")
        })
        .collect();

    // Sort by modification time, newest first
    sockets.sort_by(|a, b| {
        let ta = a.metadata().and_then(|m| m.modified()).ok();
        let tb = b.metadata().and_then(|m| m.modified()).ok();
        tb.cmp(&ta)
    });

    // Return the first socket that's actually connectable
    for entry in sockets {
        let path = entry.path();
        if std::os::unix::net::UnixStream::connect(&path).is_ok() {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}

fn print_result(value: &serde_json::Value) {
    match value {
        serde_json::Value::String(s) => println!("{s}"),
        serde_json::Value::Array(arr) => {
            for item in arr {
                println!("{}", serde_json::to_string(item).unwrap());
            }
        }
        other => println!("{}", serde_json::to_string_pretty(other).unwrap()),
    }
}
