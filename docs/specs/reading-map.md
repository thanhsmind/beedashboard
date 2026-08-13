# Reading Map

Where each area of this project lives. bee-scribing owns this file: it is
updated whenever an area spec is created or moved. Read this before any broad
search — it answers "where does X live" without a grep.

| Area | Spec | Code entry points |
|---|---|---|
| Settings | `docs/specs/settings.md` | `crates/waggledance-core/src/config.rs`, `crates/waggledance/src/server.rs`, `crates/waggledance/src/views.rs`, `crates/waggledance/src/runtime.rs` |
| Doctor | `docs/specs/doctor.md` | `crates/waggledance/src/doctor.rs`, `crates/waggledance/src/cli.rs` |
| Daemon lifecycle | `docs/specs/daemon.md` | `crates/waggledance/src/runtime.rs`, `crates/waggledance-core/src/daemon.rs`, `crates/waggledance-core/src/process.rs`, `crates/waggledance/src/server.rs`, `crates/waggledance/src/cli.rs`, `crates/waggledance-desktop/src/main.rs` |
| Web interface (nav chrome) | `docs/specs/web-interface.md` | `crates/waggledance/src/views.rs`, `crates/waggledance/assets/app.js`, `crates/waggledance/assets/app.css`, `crates/waggledance/assets/atelier/components.css` |
| Bee cockpit (read-only bee dashboard per project) | `docs/specs/bee-cockpit.md` | `crates/waggledance-core/src/bee.rs`, `crates/waggledance/src/server.rs`, `crates/waggledance/src/views.rs` |
| Appearance (visual style + Light/Dark scheme) | `docs/specs/appearance.md` | `crates/waggledance/assets/atelier/`, `crates/waggledance/assets/app.css`, `crates/waggledance/src/views.rs`, `crates/waggledance/assets/app.js`, `crates/waggledance/src/server.rs`, `crates/waggledance-desktop/ui/index.html` |
| Agent terminal (per-project Terminal/Transcript tabs; gated by its own switch, no credential) | `docs/specs/agent-terminal.md` | `crates/waggledance/src/server.rs`, `crates/waggledance/src/views.rs`, `crates/waggledance/src/herdr/`, `crates/waggledance/src/supervisor.rs`, `crates/waggledance/src/notify/`, `crates/waggledance-core/src/config.rs`, `crates/waggledance-core/src/transcript.rs`, `crates/waggledance-core/src/paths_boundary.rs`, `crates/waggledance-core/src/notify_store.rs`, `crates/waggledance-core/src/ansi.rs` |
