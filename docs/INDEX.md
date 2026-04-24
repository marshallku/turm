# turm Documentation Index

## File Structure

| File                                       | Purpose                                     | When to Read                              |
| ------------------------------------------ | ------------------------------------------- | ----------------------------------------- |
| [architecture.md](./architecture.md)       | Project structure, crate layout, tech stack | Starting work, understanding the codebase |
| [linux-app.md](./linux-app.md)             | GTK4 + VTE4 Linux app internals             | Working on turm-linux                     |
| [macos-app.md](./macos-app.md)             | Swift/AppKit + SwiftTerm macOS app          | Working on turm-macos                     |
| [core-lib.md](./core-lib.md)               | Shared Rust core library modules            | Working on turm-core                      |
| [cli.md](./cli.md)                         | CLI tool (turmctl) and D-Bus interface      | Working on remote control features        |
| [config.md](./config.md)                   | Configuration format and defaults           | Adding config options                     |
| [decisions.md](./decisions.md)             | Key technical decisions and rationale       | Understanding "why" behind choices        |
| [troubleshooting.md](./troubleshooting.md) | Known issues, fixes, gotchas                | Debugging problems                        |
| [plugins.md](./plugins.md)                 | Plugin development guide + JS bridge API    | Creating plugins                          |
| [workflow-runtime.md](./workflow-runtime.md) | Event Bus, Action Registry, Context Service design | Designing integrations, triggers, AI context |
| [roadmap.md](./roadmap.md)                 | Implementation phases, pending work         | Planning next steps                       |

## Quick Reference

- **Binary names**: `turm` (terminal app), `turmctl` (CLI control tool)
- **Config path**: `~/.config/turm/config.toml`
- **Cache path**: `~/.cache/turm/wallpapers.txt`
- **GTK app ID**: `com.marshall.turm`
- **Theme**: Catppuccin Mocha
- **Rust edition**: 2024
