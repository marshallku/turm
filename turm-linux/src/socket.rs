use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use gtk4::ApplicationWindow;
use serde_json::json;

use turm_core::protocol::{Event, Request, Response};

use vte4::prelude::*;

use crate::tabs::TabManager;

const WALLPAPER_CACHE: &str = ".cache/terminal-wallpapers.txt";
const BG_MODE_FILE: &str = ".cache/turm-bg-mode";

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
            eprintln!("[turm] failed to bind socket at {socket_path}: {e}");
            return rx;
        }
    };

    eprintln!("[turm] socket server listening at {socket_path}");

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[turm] socket accept error: {e}");
                    continue;
                }
            };

            let tx = tx.clone();
            let event_bus = event_bus.clone();
            std::thread::spawn(move || {
                let reader = match stream.try_clone() {
                    Ok(s) => BufReader::new(s),
                    Err(e) => {
                        eprintln!("[turm] socket clone error: {e}");
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

/// Dispatch consumes the SocketCommand so async handlers (webview.execute_js) can
/// capture the reply sender and respond from a callback.
pub fn dispatch(cmd: SocketCommand, mgr: &Rc<TabManager>, window: &ApplicationWindow) {
    let req = &cmd.request;
    match req.method.as_str() {
        "system.ping" => {
            let _ = cmd
                .reply
                .send(Response::success(req.id.clone(), json!({ "status": "ok" })));
        }

        "background.set" => {
            let resp = handle_bg_set(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "background.clear" => {
            let resp = handle_bg_clear(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "background.next" => {
            let resp = handle_bg_next(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "background.toggle" => {
            let resp = handle_bg_toggle(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "background.set_tint" => {
            let resp = handle_bg_set_tint(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "tab.new" => {
            mgr.add_tab(window);
            let _ = cmd
                .reply
                .send(Response::success(req.id.clone(), json!({ "status": "ok" })));
        }

        "tab.close" => {
            mgr.close_focused(window);
            let _ = cmd
                .reply
                .send(Response::success(req.id.clone(), json!({ "status": "ok" })));
        }

        "tab.list" => {
            let count = mgr.tab_count();
            let current = mgr.current_tab();
            let _ = cmd.reply.send(Response::success(
                req.id.clone(),
                json!({ "count": count, "current": current }),
            ));
        }

        "tab.info" => {
            let _ = cmd
                .reply
                .send(Response::success(req.id.clone(), mgr.tab_info()));
        }

        "split.horizontal" => {
            mgr.split_focused(gtk4::Orientation::Horizontal, window);
            let _ = cmd
                .reply
                .send(Response::success(req.id.clone(), json!({ "status": "ok" })));
        }

        "split.vertical" => {
            mgr.split_focused(gtk4::Orientation::Vertical, window);
            let _ = cmd
                .reply
                .send(Response::success(req.id.clone(), json!({ "status": "ok" })));
        }

        "session.list" => {
            let _ = cmd.reply.send(Response::success(
                req.id.clone(),
                json!(mgr.all_panels_info()),
            ));
        }

        "session.info" => {
            let resp = match req.params.get("id").and_then(|v| v.as_str()) {
                Some(id) => match mgr.panel_info_by_id(id) {
                    Some(info) => Response::success(req.id.clone(), info),
                    None => Response::error(
                        req.id.clone(),
                        "not_found",
                        &format!("Panel not found: {id}"),
                    ),
                },
                None => Response::error(req.id.clone(), "invalid_params", "Missing 'id' param"),
            };
            let _ = cmd.reply.send(resp);
        }

        // -- WebView commands --
        "webview.open" => {
            let resp = handle_webview_open(req, mgr, window);
            let _ = cmd.reply.send(resp);
        }

        "webview.navigate" => {
            let resp = handle_webview_navigate(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "webview.back" => {
            let resp = with_webview_panel(req, mgr, |wv| {
                wv.go_back();
                Response::success(req.id.clone(), json!({ "status": "ok" }))
            });
            let _ = cmd.reply.send(resp);
        }

        "webview.forward" => {
            let resp = with_webview_panel(req, mgr, |wv| {
                wv.go_forward();
                Response::success(req.id.clone(), json!({ "status": "ok" }))
            });
            let _ = cmd.reply.send(resp);
        }

        "webview.reload" => {
            let resp = with_webview_panel(req, mgr, |wv| {
                wv.reload();
                Response::success(req.id.clone(), json!({ "status": "ok" }))
            });
            let _ = cmd.reply.send(resp);
        }

        "webview.execute_js" => {
            handle_webview_execute_js(cmd, mgr);
            // Response sent from callback
        }

        "webview.get_content" => {
            handle_webview_get_content(cmd, mgr);
            // Response sent from callback
        }

        "webview.screenshot" => {
            handle_webview_screenshot(cmd, mgr);
            // Response sent from callback
        }

        "webview.query" => {
            handle_webview_query(cmd, mgr);
            // Response sent from callback
        }

        "webview.query_all" => {
            handle_webview_query_all(cmd, mgr);
            // Response sent from callback
        }

        "webview.get_styles" => {
            handle_webview_get_styles(cmd, mgr);
            // Response sent from callback
        }

        "webview.click" => {
            handle_webview_click(cmd, mgr);
            // Response sent from callback
        }

        "webview.fill" => {
            handle_webview_fill(cmd, mgr);
            // Response sent from callback
        }

        "webview.scroll" => {
            handle_webview_scroll(cmd, mgr);
            // Response sent from callback
        }

        "webview.page_info" => {
            handle_webview_page_info(cmd, mgr);
            // Response sent from callback
        }

        "webview.devtools" => {
            let resp = handle_webview_devtools(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        // -- Tab bar commands --
        "tabs.toggle_bar" => {
            let visible = mgr.toggle_tab_bar();
            let _ = cmd.reply.send(Response::success(
                req.id.clone(),
                json!({ "visible": visible }),
            ));
        }

        "tab.rename" => {
            let resp = match (
                req.params.get("id").and_then(|v| v.as_str()),
                req.params.get("title").and_then(|v| v.as_str()),
            ) {
                (Some(id), Some(title)) => {
                    if mgr.rename_tab(id, title) {
                        Response::success(req.id.clone(), json!({ "status": "ok" }))
                    } else {
                        Response::error(
                            req.id.clone(),
                            "not_found",
                            &format!("Panel not found: {id}"),
                        )
                    }
                }
                _ => Response::error(
                    req.id.clone(),
                    "invalid_params",
                    "Missing 'id' and/or 'title' param",
                ),
            };
            let _ = cmd.reply.send(resp);
        }

        // -- Terminal agent commands --
        "terminal.read" => {
            let resp = handle_terminal_read(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "terminal.state" => {
            let resp = handle_terminal_state(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "terminal.exec" => {
            let resp = handle_terminal_exec(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "terminal.feed" => {
            let resp = handle_terminal_feed(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "terminal.history" => {
            let resp = handle_terminal_history(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "terminal.context" => {
            let resp = handle_terminal_context(req, mgr);
            let _ = cmd.reply.send(resp);
        }

        "agent.approve" => {
            handle_agent_approve(cmd, window);
        }

        _ => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "unknown_method",
                &format!("Unknown method: {}", req.method),
            ));
        }
    }
}

// -- Background helpers (with terminal panel type check) --

fn active_terminal_panel(
    req: &Request,
    mgr: &Rc<TabManager>,
) -> Result<Rc<crate::panel::PanelVariant>, Response> {
    match mgr.active_panel() {
        Some(panel) => {
            if panel.as_terminal().is_some() {
                Ok(panel)
            } else {
                Err(Response::error(
                    req.id.clone(),
                    "wrong_panel_type",
                    "Active panel is not a terminal",
                ))
            }
        }
        None => Err(Response::error(
            req.id.clone(),
            "no_panel",
            "No active panel",
        )),
    }
}

fn handle_bg_set(req: &Request, mgr: &Rc<TabManager>) -> Response {
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
            match active_terminal_panel(req, mgr) {
                Ok(panel) => {
                    panel.as_terminal().unwrap().set_background(path);
                    Response::success(req.id.clone(), json!({ "status": "ok" }))
                }
                Err(e) => e,
            }
        }
        None => Response::error(req.id.clone(), "invalid_params", "Missing 'path' param"),
    }
}

fn handle_bg_clear(req: &Request, mgr: &Rc<TabManager>) -> Response {
    match active_terminal_panel(req, mgr) {
        Ok(panel) => {
            panel.as_terminal().unwrap().clear_background();
            Response::success(req.id.clone(), json!({ "status": "ok" }))
        }
        Err(e) => e,
    }
}

fn handle_bg_next(req: &Request, mgr: &Rc<TabManager>) -> Response {
    if !is_bg_active() {
        return Response::success(
            req.id.clone(),
            json!({ "status": "ok", "mode": "deactive" }),
        );
    }
    match select_random_image() {
        Some(img) => {
            let path = Path::new(&img);
            if !path.exists() {
                return Response::error(
                    req.id.clone(),
                    "not_found",
                    &format!("File not found: {img}"),
                );
            }
            match active_terminal_panel(req, mgr) {
                Ok(panel) => {
                    panel.as_terminal().unwrap().set_background(path);
                    Response::success(req.id.clone(), json!({ "status": "ok", "path": img }))
                }
                Err(e) => e,
            }
        }
        None => Response::error(req.id.clone(), "no_images", "No images in wallpaper cache"),
    }
}

fn handle_bg_toggle(req: &Request, mgr: &Rc<TabManager>) -> Response {
    let now_active = toggle_bg_mode();
    if let Some(panel) = mgr.active_panel()
        && let Some(term) = panel.as_terminal()
    {
        if now_active {
            if let Some(img) = select_random_image() {
                term.set_background(Path::new(&img));
            }
        } else {
            term.clear_background();
        }
    }
    let mode = if now_active { "active" } else { "deactive" };
    Response::success(req.id.clone(), json!({ "status": "ok", "mode": mode }))
}

fn handle_bg_set_tint(req: &Request, mgr: &Rc<TabManager>) -> Response {
    let opacity = req.params.get("opacity").and_then(|v| v.as_f64());
    match opacity {
        Some(o) => match active_terminal_panel(req, mgr) {
            Ok(panel) => {
                panel.as_terminal().unwrap().set_tint(o);
                Response::success(req.id.clone(), json!({ "status": "ok" }))
            }
            Err(e) => e,
        },
        None => Response::error(req.id.clone(), "invalid_params", "Missing 'opacity' param"),
    }
}

// -- WebView command helpers --

fn handle_webview_open(
    req: &Request,
    mgr: &Rc<TabManager>,
    window: &ApplicationWindow,
) -> Response {
    let url = match req.params.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return Response::error(req.id.clone(), "invalid_params", "Missing 'url' param"),
    };
    let mode = req
        .params
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("tab");

    let panel_id = match mode {
        "split_h" => match mgr.split_focused_webview(url, gtk4::Orientation::Horizontal, window) {
            Some(id) => id,
            None => {
                return Response::error(req.id.clone(), "no_panel", "No focused panel to split");
            }
        },
        "split_v" => match mgr.split_focused_webview(url, gtk4::Orientation::Vertical, window) {
            Some(id) => id,
            None => {
                return Response::error(req.id.clone(), "no_panel", "No focused panel to split");
            }
        },
        _ => mgr.add_webview_tab(url, window),
    };

    Response::success(req.id.clone(), json!({ "panel_id": panel_id }))
}

fn handle_webview_navigate(req: &Request, mgr: &Rc<TabManager>) -> Response {
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return Response::error(req.id.clone(), "invalid_params", "Missing 'id' param"),
    };
    let url = match req.params.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return Response::error(req.id.clone(), "invalid_params", "Missing 'url' param"),
    };

    match mgr.find_panel_by_id(id) {
        Some(panel) => match panel.as_webview() {
            Some(wv) => {
                wv.navigate(url);
                Response::success(req.id.clone(), json!({ "status": "ok" }))
            }
            None => Response::error(req.id.clone(), "wrong_panel_type", "Panel is not a webview"),
        },
        None => Response::error(
            req.id.clone(),
            "not_found",
            &format!("Panel not found: {id}"),
        ),
    }
}

fn with_webview_panel(
    req: &Request,
    mgr: &Rc<TabManager>,
    f: impl FnOnce(&crate::webview::WebViewPanel) -> Response,
) -> Response {
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return Response::error(req.id.clone(), "invalid_params", "Missing 'id' param"),
    };
    match mgr.find_panel_by_id(id) {
        Some(panel) => match panel.as_webview() {
            Some(wv) => f(wv),
            None => Response::error(req.id.clone(), "wrong_panel_type", "Panel is not a webview"),
        },
        None => Response::error(
            req.id.clone(),
            "not_found",
            &format!("Panel not found: {id}"),
        ),
    }
}

fn handle_webview_execute_js(cmd: SocketCommand, mgr: &Rc<TabManager>) {
    let req = &cmd.request;
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "invalid_params",
                "Missing 'id' param",
            ));
            return;
        }
    };
    let code = match req.params.get("code").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "invalid_params",
                "Missing 'code' param",
            ));
            return;
        }
    };

    let panel = match mgr.find_panel_by_id(&id) {
        Some(p) => p,
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "not_found",
                &format!("Panel not found: {id}"),
            ));
            return;
        }
    };
    let wv = match panel.as_webview() {
        Some(wv) => wv,
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "wrong_panel_type",
                "Panel is not a webview",
            ));
            return;
        }
    };

    let req_id = req.id.clone();
    let reply = cmd.reply;
    wv.execute_js(&code, move |result| {
        let resp = match result {
            Ok(value) => Response::success(req_id, json!({ "result": value })),
            Err(e) => Response::error(req_id, "js_error", &e),
        };
        let _ = reply.send(resp);
    });
}

fn handle_webview_get_content(cmd: SocketCommand, mgr: &Rc<TabManager>) {
    let req = &cmd.request;
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "invalid_params",
                "Missing 'id' param",
            ));
            return;
        }
    };
    let format = req
        .params
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("text");
    let js_code = match format {
        "html" => "document.documentElement.outerHTML".to_string(),
        _ => "document.body.innerText".to_string(),
    };

    let panel = match mgr.find_panel_by_id(&id) {
        Some(p) => p,
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "not_found",
                &format!("Panel not found: {id}"),
            ));
            return;
        }
    };
    let wv = match panel.as_webview() {
        Some(wv) => wv,
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "wrong_panel_type",
                "Panel is not a webview",
            ));
            return;
        }
    };

    let req_id = req.id.clone();
    let reply = cmd.reply;
    wv.execute_js(&js_code, move |result| {
        let resp = match result {
            Ok(content) => Response::success(req_id, json!({ "content": content })),
            Err(e) => Response::error(req_id, "js_error", &e),
        };
        let _ = reply.send(resp);
    });
}

fn handle_webview_screenshot(cmd: SocketCommand, mgr: &Rc<TabManager>) {
    let req = &cmd.request;
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "invalid_params",
                "Missing 'id' param",
            ));
            return;
        }
    };

    let panel = match mgr.find_panel_by_id(&id) {
        Some(p) => p,
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "not_found",
                &format!("Panel not found: {id}"),
            ));
            return;
        }
    };
    let wv = match panel.as_webview() {
        Some(wv) => wv,
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "wrong_panel_type",
                "Panel is not a webview",
            ));
            return;
        }
    };

    let req_id = req.id.clone();
    let reply = cmd.reply;
    let path = req
        .params
        .get("path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    wv.snapshot(move |result| {
        let resp = match result {
            Ok(base64_png) => {
                if let Some(path) = path {
                    // Decode and save to file
                    match gtk4::glib::base64_decode(&base64_png) {
                        data if !data.is_empty() => match std::fs::write(&path, &data) {
                            Ok(_) => Response::success(req_id, json!({ "path": path })),
                            Err(e) => Response::error(req_id, "io_error", &e.to_string()),
                        },
                        _ => Response::error(req_id, "decode_error", "Failed to decode PNG"),
                    }
                } else {
                    Response::success(req_id, json!({ "image": base64_png }))
                }
            }
            Err(e) => Response::error(req_id, "snapshot_error", &e),
        };
        let _ = reply.send(resp);
    });
}

/// Helper: run a JS snippet from webview::js module on a webview panel, send result via reply
fn run_js_command(cmd: SocketCommand, mgr: &Rc<TabManager>, js_code: String) {
    let req = &cmd.request;
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "invalid_params",
                "Missing 'id' param",
            ));
            return;
        }
    };

    let panel = match mgr.find_panel_by_id(&id) {
        Some(p) => p,
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "not_found",
                &format!("Panel not found: {id}"),
            ));
            return;
        }
    };
    let wv = match panel.as_webview() {
        Some(wv) => wv,
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "wrong_panel_type",
                "Panel is not a webview",
            ));
            return;
        }
    };

    let req_id = req.id.clone();
    let reply = cmd.reply;
    wv.execute_js(&js_code, move |result| {
        let resp = match result {
            Ok(json_str) => {
                // Parse the JSON string returned by JS to embed as structured data
                match serde_json::from_str::<serde_json::Value>(&json_str) {
                    Ok(val) => Response::success(req_id, json!({ "result": val })),
                    Err(_) => Response::success(req_id, json!({ "result": json_str })),
                }
            }
            Err(e) => Response::error(req_id, "js_error", &e),
        };
        let _ = reply.send(resp);
    });
}

fn handle_webview_query(cmd: SocketCommand, mgr: &Rc<TabManager>) {
    let selector = match cmd.request.params.get("selector").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                cmd.request.id.clone(),
                "invalid_params",
                "Missing 'selector' param",
            ));
            return;
        }
    };
    let js = crate::webview::js::query_selector(&selector);
    run_js_command(cmd, mgr, js);
}

fn handle_webview_query_all(cmd: SocketCommand, mgr: &Rc<TabManager>) {
    let selector = match cmd.request.params.get("selector").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                cmd.request.id.clone(),
                "invalid_params",
                "Missing 'selector' param",
            ));
            return;
        }
    };
    let limit = cmd
        .request
        .params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as u32;
    let js = crate::webview::js::query_selector_all(&selector, limit);
    run_js_command(cmd, mgr, js);
}

fn handle_webview_get_styles(cmd: SocketCommand, mgr: &Rc<TabManager>) {
    let selector = match cmd.request.params.get("selector").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                cmd.request.id.clone(),
                "invalid_params",
                "Missing 'selector' param",
            ));
            return;
        }
    };
    let properties: Vec<&str> = cmd
        .request
        .params
        .get("properties")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let js = crate::webview::js::get_styles(&selector, &properties);
    run_js_command(cmd, mgr, js);
}

fn handle_webview_click(cmd: SocketCommand, mgr: &Rc<TabManager>) {
    let selector = match cmd.request.params.get("selector").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                cmd.request.id.clone(),
                "invalid_params",
                "Missing 'selector' param",
            ));
            return;
        }
    };
    let js = crate::webview::js::click(&selector);
    run_js_command(cmd, mgr, js);
}

fn handle_webview_fill(cmd: SocketCommand, mgr: &Rc<TabManager>) {
    let selector = match cmd.request.params.get("selector").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                cmd.request.id.clone(),
                "invalid_params",
                "Missing 'selector' param",
            ));
            return;
        }
    };
    let value = match cmd.request.params.get("value").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => {
            let _ = cmd.reply.send(Response::error(
                cmd.request.id.clone(),
                "invalid_params",
                "Missing 'value' param",
            ));
            return;
        }
    };
    let js = crate::webview::js::fill(&selector, &value);
    run_js_command(cmd, mgr, js);
}

fn handle_webview_scroll(cmd: SocketCommand, mgr: &Rc<TabManager>) {
    let selector = cmd
        .request
        .params
        .get("selector")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let x = cmd
        .request
        .params
        .get("x")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    let y = cmd
        .request
        .params
        .get("y")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    let js = crate::webview::js::scroll(selector.as_deref(), x, y);
    run_js_command(cmd, mgr, js);
}

fn handle_webview_page_info(cmd: SocketCommand, mgr: &Rc<TabManager>) {
    let js = crate::webview::js::page_info();
    run_js_command(cmd, mgr, js);
}

fn handle_webview_devtools(req: &Request, mgr: &Rc<TabManager>) -> Response {
    use webkit6::prelude::WebViewExt;
    let action = req
        .params
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("show");
    with_webview_panel(req, mgr, |wv| {
        if let Some(inspector) = wv.webview.inspector() {
            match action {
                "show" => inspector.show(),
                "close" => inspector.close(),
                "attach" => inspector.attach(),
                "detach" => inspector.detach(),
                other => {
                    return Response::error(
                        req.id.clone(),
                        "invalid_params",
                        &format!("Unknown action: {other}. Use show/close/attach/detach"),
                    );
                }
            }
            Response::success(req.id.clone(), json!({ "status": "ok" }))
        } else {
            Response::error(req.id.clone(), "no_inspector", "Inspector not available")
        }
    })
}

// -- Utility functions --

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

// -- Terminal agent command helpers --

fn resolve_terminal(
    req: &Request,
    mgr: &Rc<TabManager>,
) -> Result<Rc<crate::panel::PanelVariant>, Response> {
    // If id is provided, find that specific panel; otherwise use active panel
    let panel = if let Some(id) = req.params.get("id").and_then(|v| v.as_str()) {
        mgr.find_panel_by_id(id).ok_or_else(|| {
            Response::error(
                req.id.clone(),
                "not_found",
                &format!("Panel not found: {id}"),
            )
        })?
    } else {
        mgr.active_panel()
            .ok_or_else(|| Response::error(req.id.clone(), "no_panel", "No active panel"))?
    };

    if panel.as_terminal().is_none() {
        return Err(Response::error(
            req.id.clone(),
            "wrong_panel_type",
            "Panel is not a terminal",
        ));
    }
    Ok(panel)
}

fn handle_terminal_read(req: &Request, mgr: &Rc<TabManager>) -> Response {
    let panel = match resolve_terminal(req, mgr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let term = panel.as_terminal().unwrap();

    // Optional range params
    let has_range = req.params.get("start_row").is_some();
    let text = if has_range {
        let start_row = req
            .params
            .get("start_row")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let start_col = req
            .params
            .get("start_col")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let end_row = req
            .params
            .get("end_row")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| term.terminal.row_count() as i64 - 1);
        let end_col = req
            .params
            .get("end_col")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| term.terminal.column_count() as i64 - 1);
        term.read_range(start_row, start_col, end_row, end_col)
    } else {
        term.read_screen()
    };

    let (col, row) = term.terminal.cursor_position();
    Response::success(
        req.id.clone(),
        json!({
            "text": text,
            "cursor": [row, col],
            "rows": term.terminal.row_count(),
            "cols": term.terminal.column_count(),
        }),
    )
}

fn handle_terminal_state(req: &Request, mgr: &Rc<TabManager>) -> Response {
    let panel = match resolve_terminal(req, mgr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let term = panel.as_terminal().unwrap();
    Response::success(req.id.clone(), term.state())
}

fn handle_terminal_exec(req: &Request, mgr: &Rc<TabManager>) -> Response {
    let panel = match resolve_terminal(req, mgr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let command = match req.params.get("command").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return Response::error(req.id.clone(), "invalid_params", "Missing 'command' param");
        }
    };
    let term = panel.as_terminal().unwrap();
    // Send command + newline to execute
    term.feed_input(&format!("{command}\n"));
    Response::success(req.id.clone(), json!({ "status": "ok" }))
}

fn handle_terminal_feed(req: &Request, mgr: &Rc<TabManager>) -> Response {
    let panel = match resolve_terminal(req, mgr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let text = match req.params.get("text").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return Response::error(req.id.clone(), "invalid_params", "Missing 'text' param"),
    };
    let term = panel.as_terminal().unwrap();
    // Send raw text (no newline appended)
    term.feed_input(text);
    Response::success(req.id.clone(), json!({ "status": "ok" }))
}

fn handle_terminal_history(req: &Request, mgr: &Rc<TabManager>) -> Response {
    let panel = match resolve_terminal(req, mgr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let term = panel.as_terminal().unwrap();

    // Number of scrollback lines to read (default 100)
    let lines = req
        .params
        .get("lines")
        .and_then(|v| v.as_i64())
        .unwrap_or(100);

    let row_count = term.terminal.row_count() as i64;
    let col_count = term.terminal.column_count() as i64;

    // Negative rows access scrollback in VTE
    let start_row = -lines;
    let end_row = row_count - 1;

    let text = term.read_range(start_row, 0, end_row, col_count - 1);
    Response::success(
        req.id.clone(),
        json!({
            "text": text,
            "lines_requested": lines,
            "rows": row_count,
            "cols": col_count,
        }),
    )
}

fn handle_terminal_context(req: &Request, mgr: &Rc<TabManager>) -> Response {
    let panel = match resolve_terminal(req, mgr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let term = panel.as_terminal().unwrap();

    let state = term.state();
    let screen = term.read_screen();

    // Recent scrollback (last 50 lines above visible area)
    let history_lines = req
        .params
        .get("history_lines")
        .and_then(|v| v.as_i64())
        .unwrap_or(50);
    let col_count = term.terminal.column_count() as i64;
    let history = term.read_range(-history_lines, 0, -1, col_count - 1);

    Response::success(
        req.id.clone(),
        json!({
            "state": state,
            "screen": screen,
            "history": history,
        }),
    )
}

fn handle_agent_approve(cmd: SocketCommand, window: &ApplicationWindow) {
    let req = &cmd.request;
    let title = req
        .params
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Agent Action");
    let message = match req.params.get("message").and_then(|v| v.as_str()) {
        Some(m) => m,
        None => {
            let _ = cmd.reply.send(Response::error(
                req.id.clone(),
                "invalid_params",
                "Missing 'message' param",
            ));
            return;
        }
    };
    let actions = req
        .params
        .get("actions")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec!["Approve".to_string(), "Deny".to_string()]);

    let dialog = gtk4::AlertDialog::builder()
        .modal(true)
        .message(title)
        .detail(message)
        .buttons(actions.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        .default_button(0)
        .cancel_button(actions.len() as i32 - 1)
        .build();

    let req_id = req.id.clone();
    let actions_clone = actions.clone();
    dialog.choose(Some(window), gtk4::gio::Cancellable::NONE, move |result| {
        let resp = match result {
            Ok(idx) => {
                let action = actions_clone
                    .get(idx as usize)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                let approved = idx == 0;
                Response::success(
                    req_id.clone(),
                    json!({
                        "approved": approved,
                        "action": action,
                        "index": idx,
                    }),
                )
            }
            Err(_) => Response::success(
                req_id.clone(),
                json!({
                    "approved": false,
                    "action": "cancelled",
                    "index": -1,
                }),
            ),
        };
        let _ = cmd.reply.send(resp);
    });
}
