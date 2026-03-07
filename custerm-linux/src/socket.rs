use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use gtk4::ApplicationWindow;
use serde_json::json;

use custerm_core::protocol::{Event, Request, Response};

use crate::tabs::TabManager;

const WALLPAPER_CACHE: &str = ".cache/terminal-wallpapers.txt";
const BG_MODE_FILE: &str = ".cache/custerm-bg-mode";

pub type EventBus = Arc<Mutex<Vec<mpsc::Sender<String>>>>;

pub struct SocketCommand {
    pub request: Request,
    pub reply: std::sync::mpsc::Sender<Response>,
}

pub fn new_event_bus() -> EventBus {
    Arc::new(Mutex::new(Vec::new()))
}

pub fn broadcast(bus: &EventBus, event: &Event) {
    let json = match serde_json::to_string(event) {
        Ok(j) => j,
        Err(_) => return,
    };
    let mut senders = bus.lock().unwrap();
    senders.retain(|tx| tx.send(json.clone()).is_ok());
}

pub fn start_server(socket_path: &str, event_bus: EventBus) -> mpsc::Receiver<SocketCommand> {
    let (tx, rx) = mpsc::channel();

    // Remove stale socket
    let _ = std::fs::remove_file(socket_path);

    let listener = match UnixListener::bind(socket_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[custerm] failed to bind socket at {socket_path}: {e}");
            return rx;
        }
    };

    eprintln!("[custerm] socket server listening at {socket_path}");

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[custerm] socket accept error: {e}");
                    continue;
                }
            };

            let tx = tx.clone();
            let event_bus = event_bus.clone();
            std::thread::spawn(move || {
                let reader = match stream.try_clone() {
                    Ok(s) => BufReader::new(s),
                    Err(e) => {
                        eprintln!("[custerm] socket clone error: {e}");
                        return;
                    }
                };
                let mut writer = stream;

                for line in reader.lines() {
                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break,
                    };
                    if line.is_empty() {
                        continue;
                    }

                    let request: Request = match serde_json::from_str(&line) {
                        Ok(r) => r,
                        Err(e) => {
                            let err = Response::error(
                                String::new(),
                                "parse_error",
                                &format!("Invalid JSON: {e}"),
                            );
                            let _ = writeln!(writer, "{}", serde_json::to_string(&err).unwrap());
                            let _ = writer.flush();
                            continue;
                        }
                    };

                    // Handle event.subscribe in the socket thread (long-lived connection)
                    if request.method == "event.subscribe" {
                        let resp = Response::success(
                            request.id.clone(),
                            json!({ "status": "subscribed" }),
                        );
                        let _ = writeln!(writer, "{}", serde_json::to_string(&resp).unwrap());
                        let _ = writer.flush();

                        let (etx, erx) = mpsc::channel();
                        event_bus.lock().unwrap().push(etx);

                        for event_json in erx.iter() {
                            if writeln!(writer, "{event_json}").is_err() {
                                break;
                            }
                            if writer.flush().is_err() {
                                break;
                            }
                        }
                        return;
                    }

                    let (reply_tx, reply_rx) = mpsc::channel();
                    let cmd = SocketCommand {
                        request,
                        reply: reply_tx,
                    };

                    if tx.send(cmd).is_err() {
                        break;
                    }

                    match reply_rx.recv() {
                        Ok(response) => {
                            let _ =
                                writeln!(writer, "{}", serde_json::to_string(&response).unwrap());
                            let _ = writer.flush();
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    });

    rx
}

pub fn dispatch(
    req: &Request,
    mgr: &Rc<TabManager>,
    window: &ApplicationWindow,
) -> Response {
    match req.method.as_str() {
        "system.ping" => Response::success(req.id.clone(), json!({ "status": "ok" })),

        "background.set" => {
            let path = req.params.get("path").and_then(|v| v.as_str());
            match path {
                Some(p) => {
                    let path = Path::new(p);
                    if !path.exists() {
                        return Response::error(
                            req.id.clone(),
                            "not_found",
                            &format!("File not found: {p}"),
                        );
                    }
                    if let Some(panel) = mgr.active_panel() {
                        panel.set_background(path);
                        Response::success(req.id.clone(), json!({ "status": "ok" }))
                    } else {
                        Response::error(req.id.clone(), "no_panel", "No active panel")
                    }
                }
                None => Response::error(req.id.clone(), "invalid_params", "Missing 'path' param"),
            }
        }

        "background.clear" => {
            if let Some(panel) = mgr.active_panel() {
                panel.clear_background();
                Response::success(req.id.clone(), json!({ "status": "ok" }))
            } else {
                Response::error(req.id.clone(), "no_panel", "No active panel")
            }
        }

        "background.next" => {
            if !is_bg_active() {
                return Response::success(req.id.clone(), json!({ "status": "ok", "mode": "deactive" }));
            }
            match select_random_image() {
                Some(img) => {
                    let path = Path::new(&img);
                    if !path.exists() {
                        return Response::error(req.id.clone(), "not_found", &format!("File not found: {img}"));
                    }
                    if let Some(panel) = mgr.active_panel() {
                        panel.set_background(path);
                        Response::success(req.id.clone(), json!({ "status": "ok", "path": img }))
                    } else {
                        Response::error(req.id.clone(), "no_panel", "No active panel")
                    }
                }
                None => Response::error(req.id.clone(), "no_images", "No images in wallpaper cache"),
            }
        }

        "background.toggle" => {
            let now_active = toggle_bg_mode();
            if let Some(panel) = mgr.active_panel() {
                if now_active {
                    if let Some(img) = select_random_image() {
                        panel.set_background(Path::new(&img));
                    }
                } else {
                    panel.clear_background();
                }
            }
            let mode = if now_active { "active" } else { "deactive" };
            Response::success(req.id.clone(), json!({ "status": "ok", "mode": mode }))
        }

        "background.set_tint" => {
            let opacity = req.params.get("opacity").and_then(|v| v.as_f64());
            match opacity {
                Some(o) => {
                    if let Some(panel) = mgr.active_panel() {
                        panel.set_tint(o);
                        Response::success(req.id.clone(), json!({ "status": "ok" }))
                    } else {
                        Response::error(req.id.clone(), "no_panel", "No active panel")
                    }
                }
                None => {
                    Response::error(req.id.clone(), "invalid_params", "Missing 'opacity' param")
                }
            }
        }

        "tab.new" => {
            mgr.add_tab(window);
            Response::success(req.id.clone(), json!({ "status": "ok" }))
        }

        "tab.close" => {
            mgr.close_focused(window);
            Response::success(req.id.clone(), json!({ "status": "ok" }))
        }

        "tab.list" => {
            let count = mgr.tab_count();
            let current = mgr.current_tab();
            Response::success(
                req.id.clone(),
                json!({ "count": count, "current": current }),
            )
        }

        "tab.info" => {
            Response::success(req.id.clone(), mgr.tab_info())
        }

        "split.horizontal" => {
            mgr.split_focused(gtk4::Orientation::Horizontal, window);
            Response::success(req.id.clone(), json!({ "status": "ok" }))
        }

        "split.vertical" => {
            mgr.split_focused(gtk4::Orientation::Vertical, window);
            Response::success(req.id.clone(), json!({ "status": "ok" }))
        }

        "session.list" => {
            Response::success(req.id.clone(), json!(mgr.all_panels_info()))
        }

        "session.info" => {
            let id = req.params.get("id").and_then(|v| v.as_str());
            match id {
                Some(id) => {
                    match mgr.panel_info_by_id(id) {
                        Some(info) => Response::success(req.id.clone(), info),
                        None => Response::error(req.id.clone(), "not_found", &format!("Panel not found: {id}")),
                    }
                }
                None => Response::error(req.id.clone(), "invalid_params", "Missing 'id' param"),
            }
        }

        _ => Response::error(
            req.id.clone(),
            "unknown_method",
            &format!("Unknown method: {}", req.method),
        ),
    }
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn select_random_image() -> Option<String> {
    let cache_path = home_dir().join(WALLPAPER_CACHE);
    let content = std::fs::read_to_string(cache_path).ok()?;
    let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize;
    Some(lines[seed % lines.len()].to_string())
}

fn is_bg_active() -> bool {
    let mode_path = home_dir().join(BG_MODE_FILE);
    match std::fs::read_to_string(mode_path) {
        Ok(s) => s.trim() != "deactive",
        Err(_) => true,
    }
}

fn toggle_bg_mode() -> bool {
    let mode_path = home_dir().join(BG_MODE_FILE);
    let new_active = !is_bg_active();
    let mode = if new_active { "active" } else { "deactive" };
    let _ = std::fs::write(mode_path, mode);
    new_active
}

pub fn cleanup(socket_path: &str) {
    let _ = std::fs::remove_file(socket_path);
}
