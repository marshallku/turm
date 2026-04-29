//! Per-plugin ergonomic CLI subcommands (Phase 19.1).
//!
//! Each module here exposes a clap `Subcommand` enum + a `dispatch`
//! entrypoint that builds the right action params, optionally
//! preflight-resolves user-supplied id shorthands, calls the action
//! via the shared socket client, and renders the response — either
//! as JSON (`--json`) or in a human-readable form tailored to the
//! subcommand. Every subcommand is otherwise just sugar over the
//! generic `turmctl call <action> --params '{...}'` path; no new
//! IPC, no new actions, no plugin-side work.
//!
//! Slice 19.1a ships `todo`. 19.1b adds `kb` / `calendar`. 19.1c adds
//! `slack` / `git`. `jira` waits on Phase 16. Per-module structure
//! lets each surface evolve independently — naming conventions,
//! prefix-resolution rules, output formats — without bloating
//! `commands.rs`.

pub mod todo;
