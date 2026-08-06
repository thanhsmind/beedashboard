# Reading Map

Where each area of this project lives. bee-scribing owns this file: it is
updated whenever an area spec is created or moved. Read this before any broad
search — it answers "where does X live" without a grep.

| Area | Spec | Code entry points |
|---|---|---|
| Settings | `docs/specs/settings.md` | `crates/mdview-core/src/config.rs`, `crates/mdview/src/server.rs`, `crates/mdview/src/views.rs`, `crates/mdview/src/runtime.rs` |
| Doctor | `docs/specs/doctor.md` | `crates/mdview/src/doctor.rs`, `crates/mdview/src/cli.rs` |
| Daemon lifecycle | `docs/specs/daemon.md` | `crates/mdview/src/runtime.rs`, `crates/mdview-core/src/daemon.rs`, `crates/mdview-core/src/process.rs`, `crates/mdview/src/server.rs`, `crates/mdview/src/cli.rs`, `crates/mdview-desktop/src/main.rs` |
| Web interface (nav chrome) | `docs/specs/web-interface.md` | `crates/mdview/src/views.rs`, `crates/mdview/assets/app.js`, `crates/mdview/assets/app.css`, `crates/mdview/assets/atelier/components.css` |
| Bee cockpit (read-only bee dashboard per project) | `docs/specs/bee-cockpit.md` | `crates/mdview-core/src/bee.rs`, `crates/mdview/src/server.rs`, `crates/mdview/src/views.rs` |
| Appearance (visual style + Light/Dark scheme) | `docs/specs/appearance.md` | `crates/mdview/assets/atelier/`, `crates/mdview/assets/app.css`, `crates/mdview/src/views.rs`, `crates/mdview/assets/app.js`, `crates/mdview/src/server.rs`, `crates/mdview-desktop/ui/index.html` |
| Agent terminal (per-project Terminal/Transcript tabs; the only gated surface) | `docs/specs/agent-terminal.md` | `crates/mdview/src/server.rs`, `crates/mdview/src/terminal_auth.rs`, `crates/mdview/src/views.rs`, `crates/mdview/src/herdr/`, `crates/mdview/src/supervisor.rs`, `crates/mdview/src/notify/`, `crates/mdview-core/src/config.rs`, `crates/mdview-core/src/transcript.rs`, `crates/mdview-core/src/paths_boundary.rs`, `crates/mdview-core/src/notify_store.rs`, `crates/mdview-core/src/ansi.rs` |
