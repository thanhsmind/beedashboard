---
type: bee.delivery
title: waggledance-rename — delivery
description: "Delivery record for work item waggledance-rename: 9 capped cells renaming mdview to waggledance across every contract surface, plus a one-time data-directory migration."
timestamp: 2026-08-13
bee:
  id: waggledance-rename-delivery
  lifecycle: active
  required_context: [docs/history/waggledance-rename/CONTEXT.md, docs/history/waggledance-rename/plan.md]
  sources: [docs/history/waggledance-rename/CONTEXT.md, docs/history/waggledance-rename/plan.md, .bee/cells/waggledance-rename-1.json, .bee/cells/waggledance-rename-2.json, .bee/cells/waggledance-rename-3.json, .bee/cells/waggledance-rename-4.json, .bee/cells/waggledance-rename-5.json, .bee/cells/waggledance-rename-6.json, .bee/cells/waggledance-rename-7.json, .bee/cells/waggledance-rename-8.json, .bee/cells/waggledance-rename-9.json]
---

# waggledance-rename — Delivery

The product is **Waggle Dance**; the command is `waggledance`. The rename was sliced by
**contract surface**, not by directory — one cell per thing that could break something
outside this repository.

## What shipped

- **-1 · Crate and binary identity.** Both workspace crates moved by `git mv`, taking
  package names, the `[[bin]]` name, the dependency edge, 11 `use waggledance_core::`
  imports, the clap command name, the release `-p` flag, the repository URL, and the root
  lockfile with them.
- **-2 · Data directory and its migration.** The data directory is `~/.waggledance`, and
  `~/.mdview` migrates into it once per process **from inside the resolver**. The race
  loser treats an already-vanished source as success. The attach cache moved to
  `~/.cache/waggledance` with no migration, deliberately — nothing there outlives a session.
- **-3 · MCP tool and doctor's sweep.** The tool is `waggledance_view_file`. Doctor now
  deletes the old entry in the same write that adds the new one, across all three config
  formats; replaces an existing marker block in place instead of appending a second;
  removes the orphaned `.claude/skills/mdview/` directory; and refuses to write a
  `~/.claude.json` it cannot parse rather than rewriting it as an empty object.
- **-4 · Env vars and installers.** `WAGGLEDANCE_HERDR_BINARY`; both installers pull
  `thanhsmind/waggledance` and place the binary **outside** the config directory.
- **-5 · Web UI.** Storage keys are `waggledance-*` with a one-shot migration that keeps an
  existing reader their theme and folder state; the mermaid-done event renamed on both
  sides at once; the display brand is Waggle Dance.
- **-6 · Daemon health handshake.** Both halves renamed together, and the detection check
  extracted into a shared `looks_like_daemon` predicate.
- **-7 · Desktop crate.** Renamed with its Tauri `productName` and `identifier`
  (`dev.waggledance.app`); the OS treats it as a new application, which was accepted.
- **-8 · Project marker.** Both `.waggledance.json` and `.mdview.json` resolve a project
  root, permanently.
- **-9 · The written record.** Every document, template filename and directory path;
  the managed block in `AGENTS.md`/`CLAUDE.md` matches what doctor writes.

## Verify

The declared suite is `cargo test --workspace`. It went from **886 passing before the
feature to 906 after** — the rename itself adds no tests; the twenty come from the
behaviour the rename forced into the open.

Verified live, on the machine serving `artifact.gogl.be`: the daemon runs the new binary,
`/health` answers `{"app":"waggledance","status":"ok","version":"0.5.2"}`, `~/.mdview` is
gone, and `~/.waggledance` holds the same 138 MB `registry.db` with every registered
project intact. The D2 migration ran once, on real data, losing nothing.

Three checks are structural rather than literal, and that is the point:

- The health-handshake test hits the real `/health` route and asserts the body satisfies
  `daemon.rs`'s own predicate. Neither side hardcodes the string.
- The mermaid event name is cross-checked between dispatch and listener, anchored on an
  unrelated callback name so two agreeing hardcoded copies cannot pass it.
- Doctor's registration is tested per config format against four states — old only, both,
  neither, already correct — plus a malformed file that must be refused byte-identical.

## Deviations that changed the design

- **The migration could not live where the plan put it.** The plan said daemon startup;
  review proved that unreachable, because `create_dir_all` runs unconditionally on the way
  to the registry, the config and the lock. The first command a user ran would have created
  the new directory and disarmed the guard forever, orphaning the old registry in silence.
  It moved into the resolver.
- **Opt-in beat opt-out.** The cell specified a test-suite opt-out. The worker built an
  opt-in armed only inside `cli::run`, the one dispatch point every subcommand passes
  through. An opt-out is one missed call site away from renaming a developer's real home
  directory, and dozens of route tests resolve the data directory.
- **The installer had to move before the migration could work.** The binary installed
  *inside* the config directory, so migrating would have meant renaming the directory
  holding the running executable — `ERROR_SHARING_VIOLATION` on Windows, where that layout
  was the default. Separating them is what made D2 possible at all.
- **One dispatched worker stalled** with its work uncommitted; the remainder — two test
  files still pointing at the old directory, which was the whole of the red — was finished
  inline rather than re-dispatched.

## Open gaps

- **The desktop crate has never been built.** `pkg-config` is absent on this machine, so
  `libdbus-sys`'s build script fails before reaching the crate's own code. The same failure
  occurs against the old manifest path, so it is not a rename defect — but "the desktop
  crate builds" is unproven and needs CI or a machine with `libwebkit2gtk` and `gtk3`.
- **`install.ps1` has never been parsed.** No `pwsh` here. Review-verified only.
- **`~/.claude.json` is alphabetically reordered on every doctor write** — `serde_json` is
  declared without `preserve_order`, so its map is a `BTreeMap`. Content survives; ordering
  and formatting do not. Known, not fixed.
- **`docs/backlog.md` line 3** is live prose still naming the old command. The rest of that
  file is a historical ledger and stays as written.

## Pointers

`crates/waggledance-core/src/config.rs` (resolver and migration),
`crates/waggledance/src/doctor.rs` (registration sweep),
`crates/waggledance-core/src/daemon.rs` (`looks_like_daemon`),
`crates/waggledance/src/cli.rs` (`PROJECT_MARKERS`).
