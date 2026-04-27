//! First-party Todo service plugin for turm.
//!
//! Markdown-checkbox files at `~/docs/todos/<workspace>/<id>.md`
//! with YAML-ish frontmatter as the source of truth — vim and git
//! are first-class clients alongside this binary's actions.
//!
//! Architecture mirrors the calendar plugin: a stdio RPC loop on
//! the main thread plus a background poller thread that emits
//! file-watcher events. We intentionally do NOT delegate writes
//! through `kb.ensure` (an inter-plugin RPC) — the atomic-create
//! primitives are 30 lines of libc and rolling them in-crate is
//! cheaper than the build-graph entanglement.
//!
//! Activation `onStartup` (rather than `onAction:todo.*`) so the
//! file-watcher is alive whenever turm is running. Otherwise an
//! external `vim` edit only surfaces as `todo.changed` after the
//! first user-initiated action, which would silently break
//! triggers like `todo.completed → slack.post_message`.
//!
//! Linux-only via `compile_error!` because `store.rs` uses
//! `renameat2(RENAME_NOREPLACE)` and `O_NOFOLLOW` — same gate as
//! `turm-plugin-kb`.

#[cfg(not(target_os = "linux"))]
compile_error!(
    "turm-plugin-todo is currently Linux-only (uses renameat2, O_NOFOLLOW, OsStrExt). \
     Same gate as turm-plugin-kb."
);

mod config;
mod store;
mod todo;
mod watcher;

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread;

use serde_json::{Value, json};

use config::Config;
use store::Store;
use todo::{Priority, Status};
use watcher::Watcher;

const PROTOCOL_VERSION: u32 = 1;

fn main() {
    let config = Arc::new(Config::from_env());
    if let Some(err) = &config.fatal_error {
        eprintln!("[todo] config error (actions will return config_error): {err}");
    }
    eprintln!(
        "[todo] root = {}, default_workspace = {}",
        config.root.display(),
        config.default_workspace
    );

    // Even with a fatal_error we still try to open the store —
    // the error already came from env validation, not the
    // filesystem, so the store itself is fine. If `Store::new`
    // ALSO fails (e.g. unwriteable docs dir), that becomes a
    // second-level fatal_error and actions error out uniformly.
    let store_result = Store::new(config.root.clone());
    let (store, store_error): (Option<Arc<Store>>, Option<String>) = match store_result {
        Ok(s) => (Some(Arc::new(s)), None),
        Err(e) => {
            let (code, msg) = e.code_message();
            eprintln!("[todo] store init failed: {code}/{msg}");
            (None, Some(format!("{code}: {msg}")))
        }
    };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let (tx, rx) = channel::<String>();
    let writer_tx = tx.clone();
    thread::spawn(move || {
        let mut out = stdout.lock();
        for line in rx.iter() {
            if writeln!(out, "{line}").is_err() || out.flush().is_err() {
                break;
            }
        }
    });

    let initialized = Arc::new(AtomicBool::new(false));

    if let Some(s) = &store {
        let watcher = Watcher::new(config.clone(), s.clone(), tx.clone(), initialized.clone());
        thread::spawn(move || watcher.run());
    }

    let reader = BufReader::new(stdin.lock());
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.is_empty() {
            continue;
        }
        let frame: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[todo] parse error: {e}");
                continue;
            }
        };
        handle_frame(
            &frame,
            &writer_tx,
            &initialized,
            &config,
            store.as_ref(),
            store_error.as_deref(),
        );
    }
}

fn handle_frame(
    frame: &Value,
    tx: &Sender<String>,
    initialized: &AtomicBool,
    config: &Config,
    store: Option<&Arc<Store>>,
    store_error: Option<&str>,
) {
    let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
    let id = frame.get("id").and_then(Value::as_str).unwrap_or("");
    let params = frame.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            let proto = params.get("protocol_version").and_then(Value::as_u64);
            if proto != Some(PROTOCOL_VERSION as u64) {
                send_error(
                    tx,
                    id,
                    "protocol_mismatch",
                    &format!("todo plugin speaks protocol {PROTOCOL_VERSION}; got {proto:?}"),
                );
                return;
            }
            send_response(
                tx,
                id,
                json!({
                    "service_version": env!("CARGO_PKG_VERSION"),
                    "provides": [
                        "todo.create",
                        "todo.list",
                        "todo.set_status",
                        "todo.start",
                        "todo.delete",
                    ],
                    "subscribes": [],
                }),
            );
        }
        "initialized" => {
            initialized.store(true, Ordering::SeqCst);
        }
        "action.invoke" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let action_params = params.get("params").cloned().unwrap_or(Value::Null);
            let result = handle_action(&name, &action_params, config, store, store_error, tx);
            match result {
                Ok(v) => send_response(tx, id, v),
                Err((code, msg)) => send_error(tx, id, &code, &msg),
            }
        }
        "event.dispatch" => {
            // No subscriptions — quietly ignore.
        }
        "shutdown" => std::process::exit(0),
        other if !other.is_empty() => {
            if !id.is_empty() {
                send_error(
                    tx,
                    id,
                    "unknown_method",
                    &format!("todo plugin: unknown method {other}"),
                );
            }
        }
        _ => {}
    }
}

fn handle_action(
    name: &str,
    params: &Value,
    config: &Config,
    store: Option<&Arc<Store>>,
    store_error: Option<&str>,
    tx: &Sender<String>,
) -> Result<Value, (String, String)> {
    if let Some(err) = &config.fatal_error {
        return Err(("config_error".into(), err.clone()));
    }
    let store = match store {
        Some(s) => s,
        None => {
            return Err((
                "config_error".into(),
                store_error
                    .map(str::to_string)
                    .unwrap_or_else(|| "todo store not initialized".to_string()),
            ));
        }
    };
    match name {
        "todo.create" => action_create(params, config, store).map(|t| {
            json!({
                "id": t.id,
                "workspace": t.workspace,
                "todo": t.to_json(),
            })
        }),
        "todo.list" => action_list(params, store),
        "todo.set_status" => action_set_status(params, config, store),
        "todo.start" => action_start(params, config, store, tx),
        "todo.delete" => action_delete(params, config, store),
        other => Err((
            "action_not_found".into(),
            format!("todo plugin does not handle {other}"),
        )),
    }
}

fn action_create(
    params: &Value,
    config: &Config,
    store: &Arc<Store>,
) -> Result<crate::todo::Todo, (String, String)> {
    let workspace = string_param_or_default(params, "workspace", &config.default_workspace)?;
    let title = required_string(params, "title")?;
    let body = optional_string(params, "body")?.unwrap_or_default();
    let priority = match optional_string(params, "priority")? {
        Some(s) => Priority::parse(&s).ok_or_else(|| {
            (
                "invalid_params".to_string(),
                format!("priority {s:?} not in {{low, normal, high}}"),
            )
        })?,
        None => Priority::Normal,
    };
    let due = optional_string(params, "due")?;
    let id = optional_string(params, "id")?;
    let linked_jira = optional_string(params, "linked_jira")?;
    let linked_slack = optional_array(params, "linked_slack")?;
    let linked_kb = optional_string_array(params, "linked_kb")?;
    let tags = optional_string_array(params, "tags")?;

    store
        .create(
            &workspace,
            id,
            &title,
            &body,
            priority,
            due,
            linked_jira,
            linked_slack,
            linked_kb,
            tags,
        )
        .map_err(|e| {
            let (c, m) = e.code_message();
            (c.to_string(), m)
        })
}

fn action_list(params: &Value, store: &Arc<Store>) -> Result<Value, (String, String)> {
    let workspace_filter = optional_string(params, "workspace")?;
    let status_filter = match optional_string(params, "status")? {
        Some(s) => Some(Status::parse(&s).ok_or_else(|| {
            (
                "invalid_params".to_string(),
                format!("status {s:?} not in {{open, in_progress, blocked, done}}"),
            )
        })?),
        None => None,
    };
    let due_before = optional_string(params, "due_before")?;
    let tag_filter = optional_string(params, "tag")?;
    let mut todos = store.list_all(workspace_filter.as_deref()).map_err(|e| {
        let (c, m) = e.code_message();
        (c.to_string(), m)
    })?;
    if let Some(target) = status_filter {
        todos.retain(|t| t.status == target);
    }
    if let Some(due_max) = &due_before {
        todos.retain(|t| match &t.due {
            Some(d) => d.as_str() < due_max.as_str(),
            None => false,
        });
    }
    if let Some(tag) = &tag_filter {
        todos.retain(|t| t.tags.iter().any(|x| x == tag));
    }
    // Stable order — by status (open first), then due, then id.
    // Easier-to-read panel without UI sorting.
    todos.sort_by(|a, b| {
        let order = |s: Status| match s {
            Status::InProgress => 0,
            Status::Open => 1,
            Status::Blocked => 2,
            Status::Done => 3,
        };
        order(a.status)
            .cmp(&order(b.status))
            .then_with(|| {
                a.due
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.due.as_deref().unwrap_or(""))
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(json!({
        "todos": todos.iter().map(crate::todo::Todo::to_json).collect::<Vec<_>>(),
    }))
}

fn action_set_status(
    params: &Value,
    config: &Config,
    store: &Arc<Store>,
) -> Result<Value, (String, String)> {
    let workspace = string_param_or_default(params, "workspace", &config.default_workspace)?;
    let id = required_string(params, "id")?;
    let status_str = required_string(params, "status")?;
    let new_status = Status::parse(&status_str).ok_or_else(|| {
        (
            "invalid_params".to_string(),
            format!("status {status_str:?} not in {{open, in_progress, blocked, done}}"),
        )
    })?;
    let (prev, next) = store.set_status(&workspace, &id, new_status).map_err(|e| {
        let (c, m) = e.code_message();
        (c.to_string(), m)
    })?;
    Ok(json!({
        "id": id,
        "workspace": workspace,
        "previous_status": prev.as_str(),
        "status": next.as_str(),
    }))
}

/// Emit a `todo.start_requested` event with the full Todo payload.
/// Phase 15.2 will hook this up to `git.worktree_add` → `claude.start`
/// via chained `<action>.completed` triggers (Phase 14.1). For now,
/// the event is fire-and-forget — users can already wire single-step
/// triggers like `todo.start_requested → kb.ensure` (write a session
/// note) or `todo.start_requested → slack.post_message` (DM
/// "starting on X") today.
fn action_start(
    params: &Value,
    config: &Config,
    store: &Arc<Store>,
    tx: &Sender<String>,
) -> Result<Value, (String, String)> {
    let workspace = string_param_or_default(params, "workspace", &config.default_workspace)?;
    let id = required_string(params, "id")?;
    let todo = store.read(&workspace, &id).map_err(|e| {
        let (c, m) = e.code_message();
        (c.to_string(), m)
    })?;
    let payload = todo.to_json();
    let frame = json!({
        "method": "event.publish",
        "params": {
            "kind": "todo.start_requested",
            "payload": payload.clone(),
        }
    });
    if let Err(e) = tx.send(frame.to_string()) {
        eprintln!("[todo] failed to enqueue todo.start_requested: {e}");
    }
    Ok(json!({
        "id": id,
        "workspace": workspace,
        "todo": payload,
    }))
}

fn action_delete(
    params: &Value,
    config: &Config,
    store: &Arc<Store>,
) -> Result<Value, (String, String)> {
    let workspace = string_param_or_default(params, "workspace", &config.default_workspace)?;
    let id = required_string(params, "id")?;
    store.delete(&workspace, &id).map_err(|e| {
        let (c, m) = e.code_message();
        (c.to_string(), m)
    })?;
    Ok(json!({ "id": id, "workspace": workspace }))
}

// -- param helpers --

fn required_string(params: &Value, key: &str) -> Result<String, (String, String)> {
    let v = params.get(key).ok_or_else(|| {
        (
            "invalid_params".to_string(),
            format!("missing required field {key:?}"),
        )
    })?;
    v.as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            (
                "invalid_params".to_string(),
                format!("{key:?} must be a non-empty string"),
            )
        })
}

fn optional_string(params: &Value, key: &str) -> Result<Option<String>, (String, String)> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err((
            "invalid_params".into(),
            format!("{key:?} must be a string, got {other}"),
        )),
    }
}

fn string_param_or_default(
    params: &Value,
    key: &str,
    default: &str,
) -> Result<String, (String, String)> {
    Ok(optional_string(params, key)?.unwrap_or_else(|| default.to_string()))
}

fn optional_array(params: &Value, key: &str) -> Result<Vec<Value>, (String, String)> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(other) => Err((
            "invalid_params".into(),
            format!("{key:?} must be an array, got {other}"),
        )),
    }
}

fn optional_string_array(params: &Value, key: &str) -> Result<Vec<String>, (String, String)> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str().map(str::to_string).ok_or((
                    "invalid_params".to_string(),
                    format!("{key:?} entries must be strings"),
                ))
            })
            .collect(),
        Some(other) => Err((
            "invalid_params".into(),
            format!("{key:?} must be an array, got {other}"),
        )),
    }
}

fn send_response(tx: &Sender<String>, id: &str, result: Value) {
    let frame = json!({ "id": id, "ok": true, "result": result });
    let _ = tx.send(frame.to_string());
}

fn send_error(tx: &Sender<String>, id: &str, code: &str, message: &str) {
    let frame = json!({
        "id": id,
        "ok": false,
        "error": { "code": code, "message": message },
    });
    let _ = tx.send(frame.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use tempfile::tempdir;

    fn fixture() -> (
        tempfile::TempDir,
        Config,
        Arc<Store>,
        Sender<String>,
        std::sync::mpsc::Receiver<String>,
    ) {
        let dir = tempdir().unwrap();
        let config = Config {
            root: dir.path().join("todos"),
            default_workspace: "default".into(),
            poll_interval: std::time::Duration::from_secs(2),
            fatal_error: None,
        };
        let store = Arc::new(Store::new(config.root.clone()).unwrap());
        let (tx, rx) = channel();
        (dir, config, store, tx, rx)
    }

    #[test]
    fn create_then_list_returns_one() {
        let (_d, config, store, _tx, _rx) = fixture();
        let created = action_create(
            &json!({"title": "ship 15.1", "priority": "high"}),
            &config,
            &store,
        )
        .unwrap();
        assert_eq!(created.title, "ship 15.1");
        assert_eq!(created.priority, Priority::High);
        let listed = action_list(&Value::Null, &store).unwrap();
        let arr = listed["todos"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["title"], "ship 15.1");
    }

    #[test]
    fn create_rejects_empty_title() {
        let (_d, config, store, _tx, _rx) = fixture();
        let err = action_create(&json!({"title": ""}), &config, &store).unwrap_err();
        assert_eq!(err.0, "invalid_params");
    }

    #[test]
    fn create_rejects_unknown_priority() {
        let (_d, config, store, _tx, _rx) = fixture();
        let err = action_create(
            &json!({"title": "x", "priority": "urgent"}),
            &config,
            &store,
        )
        .unwrap_err();
        assert_eq!(err.0, "invalid_params");
    }

    #[test]
    fn set_status_round_trip() {
        let (_d, config, store, _tx, _rx) = fixture();
        let t = action_create(&json!({"title": "x"}), &config, &store).unwrap();
        let r = action_set_status(&json!({"id": t.id, "status": "done"}), &config, &store).unwrap();
        assert_eq!(r["previous_status"], "open");
        assert_eq!(r["status"], "done");
    }

    #[test]
    fn list_filters_by_status_and_tag() {
        let (_d, config, store, _tx, _rx) = fixture();
        action_create(&json!({"title": "a", "tags": ["work"]}), &config, &store).unwrap();
        let b = action_create(
            &json!({"title": "b", "tags": ["personal"]}),
            &config,
            &store,
        )
        .unwrap();
        action_set_status(&json!({"id": b.id, "status": "done"}), &config, &store).unwrap();
        let work_open = action_list(&json!({"status": "open", "tag": "work"}), &store).unwrap();
        assert_eq!(work_open["todos"].as_array().unwrap().len(), 1);
        let all_done = action_list(&json!({"status": "done"}), &store).unwrap();
        assert_eq!(all_done["todos"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn list_filters_by_due_before() {
        let (_d, config, store, _tx, _rx) = fixture();
        action_create(
            &json!({"title": "soon", "due": "2026-04-30"}),
            &config,
            &store,
        )
        .unwrap();
        action_create(
            &json!({"title": "later", "due": "2026-06-30"}),
            &config,
            &store,
        )
        .unwrap();
        let r = action_list(&json!({"due_before": "2026-05-01"}), &store).unwrap();
        let arr = r["todos"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["title"], "soon");
    }

    #[test]
    fn start_emits_event_and_returns_payload() {
        let (_d, config, store, tx, rx) = fixture();
        let t = action_create(&json!({"title": "kickoff"}), &config, &store).unwrap();
        let r = action_start(&json!({"id": t.id}), &config, &store, &tx).unwrap();
        assert_eq!(r["todo"]["title"], "kickoff");
        let frame: Value = serde_json::from_str(&rx.recv().unwrap()).unwrap();
        assert_eq!(frame["method"], "event.publish");
        assert_eq!(frame["params"]["kind"], "todo.start_requested");
    }

    #[test]
    fn fatal_error_short_circuits_actions() {
        let (_d, _, store, tx, _rx) = fixture();
        let bad_config = Config {
            root: store.root().to_path_buf(),
            default_workspace: "default".into(),
            poll_interval: std::time::Duration::from_secs(2),
            fatal_error: Some("bogus".into()),
        };
        let err = handle_action(
            "todo.create",
            &json!({"title": "x"}),
            &bad_config,
            Some(&store),
            None,
            &tx,
        )
        .unwrap_err();
        assert_eq!(err.0, "config_error");
    }

    #[test]
    fn unknown_action_returns_action_not_found() {
        let (_d, config, store, tx, _rx) = fixture();
        let err =
            handle_action("todo.fly", &Value::Null, &config, Some(&store), None, &tx).unwrap_err();
        assert_eq!(err.0, "action_not_found");
    }
}
