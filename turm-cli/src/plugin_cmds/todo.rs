//! `turmctl todo` — ergonomic wrapper over the `todo.*` action surface.
//!
//! Maps clap subcommands onto the existing actions exposed by
//! `turm-plugin-todo`:
//!
//! | CLI                                 | Action            |
//! |-------------------------------------|-------------------|
//! | `todo create <title> [--workspace …]` | `todo.create`     |
//! | `todo list [--status …]`              | `todo.list`       |
//! | `todo set <id> --status <s>`          | `todo.set_status` |
//! | `todo done <id>`                      | `todo.set_status` (status=done) |
//! | `todo doing <id>`                     | `todo.set_status` (status=in_progress) |
//! | `todo block <id>`                     | `todo.set_status` (status=blocked) |
//! | `todo start <id>`                     | `todo.start`      |
//! | `todo delete <id>`                    | `todo.delete`     |
//!
//! ## ID prefix resolution
//!
//! Every command that takes an `<id>` accepts a unique prefix in lieu
//! of the full `T-<datetime>-<seq>` identifier. We preflight a
//! `todo.list` call, find the unique matching id, and substitute it
//! before dispatch. If zero or many ids match, we error out with the
//! candidates so the user can disambiguate. The roundtrip is cheap on
//! a local socket and saves the user from having to look up the full
//! id from the panel for routine actions.
//!
//! ## Workspace defaulting
//!
//! `--workspace` defaults to `TURM_TODO_DEFAULT_WORKSPACE` env var if
//! set, else `"default"`. Cwd-derived workspace resolution is a
//! Phase 19.X follow-up (needs `git.resolve_workspace` lifted into a
//! turm-internal action) — until then, users on multiple workspaces
//! pass `--workspace` explicitly. The plugin itself defaults to
//! `default` if the field is omitted from `todo.create`, so omitting
//! the flag here is also fine for the common case.

use clap::Subcommand;
use serde_json::{Value, json};

use crate::client;

/// Pretty status icon for a `todo.list` row. Matches the panel's
/// "Doing" column label (`in_progress` → `~`).
fn status_icon(status: &str) -> &'static str {
    match status {
        "open" => "[ ]",
        "in_progress" => "[~]",
        "done" => "[x]",
        "blocked" => "[!]",
        _ => "[?]",
    }
}

#[derive(Subcommand, Debug)]
pub enum TodoCommand {
    /// Create a new todo
    Create {
        /// Title (required)
        title: String,
        /// Body / description
        #[arg(long)]
        body: Option<String>,
        /// Workspace label (defaults to `TURM_TODO_DEFAULT_WORKSPACE` or "default")
        #[arg(long)]
        workspace: Option<String>,
        /// Priority: low | normal | high
        #[arg(long, default_value = "normal")]
        priority: String,
        /// Due date (ISO 8601, e.g. 2026-05-01)
        #[arg(long)]
        due: Option<String>,
        /// Linked Jira ticket key
        #[arg(long = "linked-jira")]
        linked_jira: Option<String>,
        /// Tags (comma-separated)
        #[arg(long)]
        tags: Option<String>,
    },
    /// List todos
    List {
        /// Filter by status (open|in_progress|done|blocked)
        #[arg(long)]
        status: Option<String>,
        /// Filter by workspace
        #[arg(long)]
        workspace: Option<String>,
        /// Filter by tag (single tag — matches todos that contain it)
        #[arg(long)]
        tag: Option<String>,
        /// Filter to todos due before the given ISO date (e.g. 2026-05-01)
        #[arg(long = "due-before")]
        due_before: Option<String>,
        /// Hide todos with status=done (default false; pass to declutter)
        #[arg(long)]
        hide_done: bool,
    },
    /// Set a todo's status (or use `done` / `doing` / `block` shorthands)
    Set {
        /// Todo id (full id or unique prefix)
        id: String,
        /// New status: open | in_progress | done | blocked
        #[arg(long)]
        status: String,
        /// Scope id resolution to this workspace (disambiguates ids that
        /// exist in multiple workspaces — todo ids are workspace-scoped,
        /// not globally unique)
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Mark a todo done (`set --status done` shorthand)
    Done {
        /// Todo id (full id or unique prefix)
        id: String,
        /// Scope id resolution to this workspace
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Move a todo to in-progress (`set --status in_progress` shorthand)
    Doing {
        /// Todo id (full id or unique prefix)
        id: String,
        /// Scope id resolution to this workspace
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Mark a todo blocked (`set --status blocked` shorthand)
    Block {
        /// Todo id (full id or unique prefix)
        id: String,
        /// Scope id resolution to this workspace
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Trigger the todo.start workflow (vision-flow-3 chain)
    Start {
        /// Todo id (full id or unique prefix)
        id: String,
        /// Scope id resolution to this workspace
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Delete a todo (irreversible)
    Delete {
        /// Todo id (full id or unique prefix)
        id: String,
        /// Scope id resolution to this workspace
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Show full Todo with linked-entity expansion (kb previews,
    /// linked Jira/Slack list)
    Show {
        /// Todo id (full id or unique prefix)
        id: String,
        /// Scope id resolution to this workspace
        #[arg(long)]
        workspace: Option<String>,
    },
}

/// Top-level dispatch. Performs id-prefix resolution where needed,
/// calls the action via the socket, and renders. Returns process
/// exit code so `main.rs` can propagate it.
pub fn dispatch(cmd: &TodoCommand, socket_path: &str, json_out: bool) -> i32 {
    match cmd {
        TodoCommand::Create {
            title,
            body,
            workspace,
            priority,
            due,
            linked_jira,
            tags,
        } => {
            let mut params = json!({
                "title": title,
                "priority": priority,
            });
            // Workspace resolution: explicit flag > env var > plugin default.
            // Cwd-derived workspace is a Phase 19.X follow-up.
            if let Some(ws) = workspace {
                params["workspace"] = json!(ws);
            } else if let Ok(ws) = std::env::var("TURM_TODO_DEFAULT_WORKSPACE") {
                params["workspace"] = json!(ws);
            }
            if let Some(b) = body {
                params["body"] = json!(b);
            }
            if let Some(d) = due {
                params["due"] = json!(d);
            }
            if let Some(j) = linked_jira {
                params["linked_jira"] = json!(j);
            }
            if let Some(t) = tags {
                let parts: Vec<&str> = t
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                params["tags"] = json!(parts);
            }
            call_and_render(socket_path, "todo.create", params, json_out, |v| {
                // Response shape: `{id, workspace, todo}` (full Todo
                // payload nested under `todo`). Render the id +
                // workspace; the rest is fetchable via `todo list` /
                // `todo show` (19.2).
                let id = v.get("id").and_then(Value::as_str).unwrap_or("?");
                let ws = v.get("workspace").and_then(Value::as_str).unwrap_or("?");
                println!("created {id} (ws={ws})");
            })
        }
        TodoCommand::List {
            status,
            workspace,
            tag,
            due_before,
            hide_done,
        } => {
            let mut params = json!({});
            if let Some(s) = status {
                params["status"] = json!(s);
            }
            if let Some(w) = workspace {
                params["workspace"] = json!(w);
            }
            if let Some(t) = tag {
                params["tag"] = json!(t);
            }
            if let Some(d) = due_before {
                params["due_before"] = json!(d);
            }
            call_and_render(socket_path, "todo.list", params, json_out, |v| {
                render_list(v, *hide_done);
            })
        }
        TodoCommand::Set {
            id,
            status,
            workspace,
        } => set_status(socket_path, id, status, workspace.as_deref(), json_out),
        TodoCommand::Done { id, workspace } => {
            set_status(socket_path, id, "done", workspace.as_deref(), json_out)
        }
        TodoCommand::Doing { id, workspace } => set_status(
            socket_path,
            id,
            "in_progress",
            workspace.as_deref(),
            json_out,
        ),
        TodoCommand::Block { id, workspace } => {
            set_status(socket_path, id, "blocked", workspace.as_deref(), json_out)
        }
        TodoCommand::Start { id, workspace } => {
            let r = match resolve_id(socket_path, id, workspace.as_deref()) {
                Ok(r) => r,
                Err(code) => return code,
            };
            call_and_render(
                socket_path,
                "todo.start",
                json!({ "id": r.id, "workspace": r.workspace }),
                json_out,
                |_| println!("started {} (ws={})", r.id, r.workspace),
            )
        }
        TodoCommand::Delete { id, workspace } => {
            let r = match resolve_id(socket_path, id, workspace.as_deref()) {
                Ok(r) => r,
                Err(code) => return code,
            };
            call_and_render(
                socket_path,
                "todo.delete",
                json!({ "id": r.id, "workspace": r.workspace }),
                json_out,
                |_| println!("deleted {} (ws={})", r.id, r.workspace),
            )
        }
        TodoCommand::Show { id, workspace } => {
            show(socket_path, id, workspace.as_deref(), json_out)
        }
    }
}

/// `turmctl todo show <id>` — full Todo + linked-entity expansion.
///
/// Composes existing actions, no new plugin work:
/// 1. Resolve id via the same preflight `todo.list` used by set/start/delete.
/// 2. For each `linked_kb` entry: call `kb.read {id}` (best-effort,
///    swallow per-entry errors). First few lines rendered as a
///    preview in human mode; `--json` returns full content.
/// 3. `linked_jira` would fan out to `jira.get_ticket` once Phase 16
///    ships; until then we render the keys verbatim.
/// 4. `linked_slack` permalinks rendered as-is — no fan-out (no
///    cheap "read message body" action that doesn't burn rate
///    limit; user can click the permalink).
/// 5. Timeline (todo.changed / todo.completed / todo.start_requested
///    history) waits on Phase 19.2c's `event.history` ring buffer —
///    until then there's no socket-callable history surface.
fn show(
    socket_path: &str,
    id_or_prefix: &str,
    workspace_filter: Option<&str>,
    json_out: bool,
) -> i32 {
    let r = match resolve_id(socket_path, id_or_prefix, workspace_filter) {
        Ok(r) => r,
        Err(code) => return code,
    };
    // Re-fetch the full todo from the resolved id. The preflight in
    // `resolve_id` already pulled `todo.list`, but it discards
    // everything except `id` and `workspace`; doing one more list
    // (filtered by workspace) is cheaper than threading the full
    // object through and doesn't depend on `resolve_id`'s internal
    // shape.
    let todo = match find_todo(socket_path, &r.id, &r.workspace) {
        Ok(t) => t,
        Err(code) => return code,
    };
    let linked_kb_arr = todo
        .get("linked_kb")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut kb_entries: Vec<KbEntry> = Vec::new();
    for kb_id in &linked_kb_arr {
        if let Some(kb_id_str) = kb_id.as_str() {
            let resp = call_one(socket_path, "kb.read", json!({ "id": kb_id_str }));
            kb_entries.push((kb_id_str.to_string(), resp));
        }
    }

    if json_out {
        let kb_json: Vec<Value> = kb_entries
            .iter()
            .map(|(id, res)| match res {
                // Pass through the full kb.read payload (content +
                // frontmatter + path + whatever else the plugin
                // adds in future) so scripts piping
                // `--json` get the same data as a direct
                // `turmctl call kb.read` would.
                Ok(v) => {
                    let mut obj = serde_json::Map::new();
                    obj.insert("id".into(), Value::String(id.clone()));
                    obj.insert("ok".into(), Value::Bool(true));
                    if let Value::Object(payload) = v {
                        for (k, v) in payload {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                    Value::Object(obj)
                }
                Err((code, msg)) => json!({ "id": id, "ok": false, "code": code, "message": msg }),
            })
            .collect();
        let aggregate = json!({
            "todo": todo,
            "linked_kb_resolved": kb_json,
        });
        println!("{}", serde_json::to_string_pretty(&aggregate).unwrap());
        return 0;
    }

    render_show(&todo, &kb_entries);
    0
}

/// Pull the full todo object from `todo.list` (workspace-filtered)
/// by id. Returns the Value for the matching entry, or an error
/// exit code after printing a diagnostic.
fn find_todo(socket_path: &str, id: &str, workspace: &str) -> Result<Value, i32> {
    let resp =
        match client::send_command(socket_path, "todo.list", json!({ "workspace": workspace })) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Error: todo.list failed: {e}");
                return Err(1);
            }
        };
    if !resp.ok {
        let err = resp
            .error
            .map(|e| format!("[{}] {}", e.code, e.message))
            .unwrap_or_default();
        eprintln!("Error: todo.list failed: {err}");
        return Err(1);
    }
    let arr = resp
        .result
        .and_then(|v| v.get("todos").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    arr.into_iter()
        .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
        .ok_or_else(|| {
            eprintln!("Error: todo {id} disappeared between resolve and fetch (concurrent edit?)");
            1
        })
}

/// Single one-shot action call used by the linked-entity fan-out.
/// Distinct from `call_and_render` because we don't want to print
/// or exit on per-call failure — `show` aggregates and renders all
/// at once.
fn call_one(socket_path: &str, method: &str, params: Value) -> Result<Value, (String, String)> {
    let resp = client::send_command(socket_path, method, params)
        .map_err(|e| ("transport_error".to_string(), e.to_string()))?;
    if resp.ok {
        Ok(resp.result.unwrap_or(Value::Null))
    } else {
        Err(resp
            .error
            .map(|e| (e.code, e.message))
            .unwrap_or_else(|| ("unknown".into(), String::new())))
    }
}

type KbEntry = (String, Result<Value, (String, String)>);

fn render_show(todo: &Value, kb_entries: &[KbEntry]) {
    let id = todo.get("id").and_then(Value::as_str).unwrap_or("?");
    let title = todo.get("title").and_then(Value::as_str).unwrap_or("");
    let status = todo.get("status").and_then(Value::as_str).unwrap_or("?");
    let priority = todo.get("priority").and_then(Value::as_str).unwrap_or("?");
    let workspace = todo.get("workspace").and_then(Value::as_str).unwrap_or("?");
    let icon = status_icon(status);

    println!("{icon} {id}  {title}");
    println!("  status={status}  priority={priority}  workspace={workspace}");

    if let Some(due) = todo.get("due").and_then(Value::as_str)
        && !due.is_empty()
    {
        println!("  due {due}");
    }
    if let Some(tags) = todo.get("tags").and_then(Value::as_array)
        && !tags.is_empty()
    {
        let names: Vec<String> = tags
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
        if !names.is_empty() {
            println!("  tags {}", names.join(", "));
        }
    }
    if let Some(jira) = todo.get("linked_jira").and_then(Value::as_str)
        && !jira.is_empty()
    {
        // jira.get_ticket fan-out lands once Phase 16 ships; until
        // then we just surface the key.
        println!("  jira {jira}");
    }
    if let Some(slack) = todo.get("linked_slack").and_then(Value::as_array)
        && !slack.is_empty()
    {
        // `linked_slack` is `Vec<Value>` per the todo schema —
        // entries can be permalink strings OR structured objects
        // (e.g. `{team, channel, ts}` matching the slack.reaction
        // payload shape). Render strings verbatim; flatten objects
        // to `key=value` pairs; fall back to JSON for anything else.
        println!("  slack");
        for s in slack {
            match s {
                Value::String(p) => println!("    {p}"),
                Value::Object(map) => {
                    let pairs: Vec<String> = map
                        .iter()
                        .map(|(k, v)| match v {
                            Value::String(s) => format!("{k}={s}"),
                            other => format!("{k}={other}"),
                        })
                        .collect();
                    println!("    {}", pairs.join("  "));
                }
                other => println!("    {other}"),
            }
        }
    }
    let body = todo.get("body").and_then(Value::as_str).unwrap_or("");
    if !body.is_empty() {
        println!();
        println!("body");
        for line in body.lines() {
            println!("  {line}");
        }
    }
    let prompt = todo.get("prompt").and_then(Value::as_str).unwrap_or("");
    if !prompt.is_empty() {
        println!();
        println!("prompt");
        for line in prompt.lines() {
            println!("  {line}");
        }
    }
    if !kb_entries.is_empty() {
        println!();
        println!("linked_kb");
        for (kb_id, res) in kb_entries {
            match res {
                Ok(v) => {
                    let content = v.get("content").and_then(Value::as_str).unwrap_or("");
                    // Strip frontmatter for the preview — the user
                    // came here for body content, not metadata they
                    // already see in the kb file itself.
                    let preview = strip_frontmatter(content);
                    println!("  {kb_id}");
                    let mut shown = 0;
                    for line in preview.lines() {
                        if shown >= 5 {
                            println!("    …");
                            break;
                        }
                        let trimmed = line.trim_end();
                        if trimmed.is_empty() && shown == 0 {
                            // Skip leading blank lines after frontmatter
                            continue;
                        }
                        println!("    {trimmed}");
                        shown += 1;
                    }
                }
                Err((code, msg)) => {
                    println!("  {kb_id}  ({code}: {msg})");
                }
            }
        }
    }
}

/// Strip a leading `---\n...\n---\n` YAML frontmatter block. Returns
/// the input unchanged if no frontmatter is present.
fn strip_frontmatter(content: &str) -> &str {
    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        return content;
    }
    let after_open = content
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or(content);
    if let Some(close_idx) = after_open.find("\n---\n") {
        let after_close = &after_open[close_idx + 5..];
        return after_close.trim_start_matches('\n');
    }
    if let Some(close_idx) = after_open.find("\n---\r\n") {
        let after_close = &after_open[close_idx + 6..];
        return after_close.trim_start_matches('\n');
    }
    content
}

fn set_status(
    socket_path: &str,
    id_or_prefix: &str,
    status: &str,
    workspace_filter: Option<&str>,
    json_out: bool,
) -> i32 {
    if !["open", "in_progress", "done", "blocked"].contains(&status) {
        eprintln!("Error: status must be one of open|in_progress|done|blocked (got {status:?})");
        return 2;
    }
    let r = match resolve_id(socket_path, id_or_prefix, workspace_filter) {
        Ok(r) => r,
        Err(code) => return code,
    };
    call_and_render(
        socket_path,
        "todo.set_status",
        json!({ "id": r.id, "workspace": r.workspace, "status": status }),
        json_out,
        |_| println!("{} → {status} (ws={})", r.id, r.workspace),
    )
}

/// Render a `todo.list` response in the human-friendly form.
/// Layout: one row per todo, status icon + id + priority + title +
/// trailing meta (workspace, tags, due, linked_jira). Aligned columns
/// for status / id / priority; title and meta are flow-spaced.
fn render_list(v: &Value, hide_done: bool) {
    let todos = match v.get("todos").and_then(Value::as_array) {
        Some(arr) => arr,
        None => {
            eprintln!("(no todos array in response)");
            return;
        }
    };
    if todos.is_empty() {
        println!("(no todos)");
        return;
    }
    // Column widths — id is the only one with significant variance.
    let id_w = todos
        .iter()
        .filter_map(|t| t.get("id").and_then(Value::as_str))
        .map(str::len)
        .max()
        .unwrap_or(20);
    let mut shown = 0usize;
    for t in todos {
        let status = t.get("status").and_then(Value::as_str).unwrap_or("?");
        if hide_done && status == "done" {
            continue;
        }
        let id = t.get("id").and_then(Value::as_str).unwrap_or("?");
        let priority = t.get("priority").and_then(Value::as_str).unwrap_or("?");
        let title = t.get("title").and_then(Value::as_str).unwrap_or("");
        let workspace = t.get("workspace").and_then(Value::as_str).unwrap_or("?");
        let icon = status_icon(status);
        let mut meta = vec![format!("ws={workspace}")];
        if let Some(due) = t.get("due").and_then(Value::as_str)
            && !due.is_empty()
        {
            meta.push(format!("due={due}"));
        }
        if let Some(tags) = t.get("tags").and_then(Value::as_array) {
            let names: Vec<String> = tags
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            if !names.is_empty() {
                meta.push(format!("tags={}", names.join(",")));
            }
        }
        if let Some(jira) = t.get("linked_jira").and_then(Value::as_str)
            && !jira.is_empty()
        {
            meta.push(format!("jira={jira}"));
        }
        println!(
            "{icon} {id:<id_w$}  {priority:<6}  {title}  ·  {meta}",
            meta = meta.join(" "),
        );
        shown += 1;
    }
    if shown == 0 {
        println!("(no todos)");
    }
}

/// Result of preflighting an `<id>` argument: the full id plus the
/// workspace it lives in. The actions all default the `workspace`
/// param to the plugin's configured default if omitted, so a todo
/// in a non-default workspace would 404 without this. Always
/// preflight the workspace alongside the id.
struct ResolvedTodo {
    id: String,
    workspace: String,
}

/// Resolve a possibly-prefixed id into the full id + its workspace.
/// Calls `todo.list` (with optional `workspace` filter) and finds:
///   - All EXACT id matches across visible workspaces. Todo ids are
///     workspace-scoped, not globally unique (the store at
///     `<root>/<workspace>/<id>.md` only checks collisions per
///     workspace), so multiple workspaces can hold the same full id.
///     If we see >1 we force the user to disambiguate via
///     `--workspace`. We do NOT silently pick whichever workspace
///     `todo.list` enumerated first — that would silently mutate the
///     wrong todo.
///   - All PREFIX matches (only consulted when there are zero exact
///     matches), again disambiguated when >1.
///
/// `workspace_filter`, if `Some`, scopes the preflight at the action
/// level so the user can disambiguate a known-duplicate id by
/// passing `--workspace <ws>`.
///
/// On miss/ambiguity prints diagnostic to stderr and returns
/// `Err(exit_code)`.
fn resolve_id(
    socket_path: &str,
    prefix: &str,
    workspace_filter: Option<&str>,
) -> Result<ResolvedTodo, i32> {
    let mut params = json!({});
    if let Some(w) = workspace_filter {
        params["workspace"] = json!(w);
    }
    let resp = match client::send_command(socket_path, "todo.list", params) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: id preflight failed: {e}");
            return Err(1);
        }
    };
    if !resp.ok {
        let err = resp
            .error
            .map(|e| format!("[{}] {}", e.code, e.message))
            .unwrap_or_default();
        eprintln!("Error: id preflight failed: {err}");
        return Err(1);
    }
    let todos = resp
        .result
        .and_then(|v| v.get("todos").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let pluck = |t: &Value| {
        let id = t.get("id").and_then(Value::as_str)?.to_string();
        let workspace = t
            .get("workspace")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_string();
        Some((id, workspace))
    };
    let mut exact: Vec<ResolvedTodo> = Vec::new();
    let mut prefix_hits: Vec<ResolvedTodo> = Vec::new();
    for t in &todos {
        let Some((id, workspace)) = pluck(t) else {
            continue;
        };
        if id == prefix {
            exact.push(ResolvedTodo { id, workspace });
        } else if id.starts_with(prefix) {
            prefix_hits.push(ResolvedTodo { id, workspace });
        }
    }
    let candidates = if !exact.is_empty() {
        exact
    } else {
        prefix_hits
    };
    let kind = if !candidates.is_empty() && candidates.iter().any(|r| r.id == prefix) {
        "id"
    } else {
        "id prefix"
    };
    match candidates.len() {
        0 => {
            eprintln!("Error: no todo matches {kind} {prefix:?}");
            Err(1)
        }
        1 => Ok(candidates.into_iter().next().unwrap()),
        _ => {
            let scope = workspace_filter
                .map(|w| format!(" (within workspace {w:?})"))
                .unwrap_or_default();
            eprintln!(
                "Error: {kind} {prefix:?} matches {} todos{scope} — disambiguate via `--workspace <ws>` or a longer prefix:",
                candidates.len()
            );
            for r in candidates {
                eprintln!("  {} (ws={})", r.id, r.workspace);
            }
            Err(1)
        }
    }
}

/// Shared call + render entrypoint. Returns process exit code.
fn call_and_render(
    socket_path: &str,
    method: &str,
    params: Value,
    json_out: bool,
    human: impl FnOnce(&Value),
) -> i32 {
    let resp = match client::send_command(socket_path, method, params) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to connect: {e}");
            return 1;
        }
    };
    if !resp.ok {
        if let Some(err) = resp.error {
            eprintln!("Error [{}]: {}", err.code, err.message);
        }
        return 1;
    }
    let result = resp.result.unwrap_or(Value::Null);
    if json_out {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        human(&result);
    }
    0
}
