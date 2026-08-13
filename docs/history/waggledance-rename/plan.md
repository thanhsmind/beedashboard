# Plan — waggledance-rename (rev 2)

Lane: **high-risk** (6 flags: public-contracts, external-systems, cross-platform,
covered-contract-change, data-model, multi-domain). ~50 product files.
Decisions: [CONTEXT.md](CONTEXT.md) D1–D9.

Rev 2 replaces rev 1 after a two-reviewer wave. Rev 1's W2 was **wrong** (migration at
daemon startup cannot fire) and its cell set missed eight surfaces, two of them contracts.
What changed is listed under "What the review changed" at the bottom.

## Shape

Rename by **contract surface**, not by directory. Each cell owns one thing that can break
something outside this repo, so each can be proved alone. Crate identity goes first because
every other cell lives inside the renamed crates.

### Intentional survivors of the word `mdview`

Each is the *reason* a cell exists, and each is pinned by a test rather than left to a grep:

1. `~/.mdview` — read once by the D2 migration (W2).
2. `mcpServers.mdview` / `[mcp_servers.mdview]` — read once by doctor's stale-entry sweep (W3).
3. `<!-- mdview:START/END -->` — the old marker, found once so the block is *replaced* not
   duplicated (W3).
4. `~/.claude/skills/mdview/` — removed once by doctor (W3).
5. `mdview-theme` / `mdview-folders-open` — read once by the D5 storage fallback (W5).
6. `.mdview.json` — permanently accepted as a project marker (D8, W8).
7. `path_hash("mdview", …)` golden constants in `short_link.rs:68-69` and the SQL fixtures in
   `repository.rs:529-583` — **not renamed at all**. There `mdview` is an arbitrary
   *registered-project name*, not this binary; rewriting the hash input would break the
   golden constants for no gain.

## Slice 1 — walking skeleton

**W1 · Crate and binary identity.**
`git mv crates/mdview-core crates/waggledance-core` and `crates/mdview crates/waggledance`.
Package names, workspace `members`, `[[bin]] name`, the dependency edge, clap
`#[command(name)]` (`cli.rs:14`), 26 `use mdview_core::` lines across 12 files,
`-p mdview` (`release.yml:62,64`), `Cargo.toml:14` `repository` → `github.com/thanhsmind/waggledance`
(D4), root `Cargo.lock` regenerated and committed.
Leaves `crates/mdview-desktop` and `.gitignore:56` alone — both belong to W6, and touching
the ignore path here would un-ignore the desktop target for the whole slice.
*Verify:* `cargo test --workspace` green; `./target/debug/waggledance --help` prints
`Usage: waggledance`.

## Slice 2 — the contract surfaces

**W2 · Config dir + one-time migration (D2, D7).** *Test-first.*
`config.rs:182 data_dir()` → `~/.waggledance`.

The migration lives **inside the resolver**, not at daemon startup. Rev 1 had this wrong:
`repository.rs:18` `create_dir_all` runs unconditionally, and so do `Config::save_to`
(`config.rs:370`) and `write_atomic` (`config.rs:380`), so the first command a user runs —
`doctor` at `:128`, any `build_engine()` caller (`runtime.rs:15`), the MCP server
(`mcp.rs:16`), even `serve` persisting its own `--port` before serving (`cli.rs:207-216`) —
creates `~/.waggledance/` and permanently disarms a "new dir absent" guard. The old registry
would be orphaned in silence.

Design: a run-once-per-process migration in `resolve_data_dir`, behind an explicit opt-out
so the suite cannot touch a developer's real home (`config.rs:431-434`
`resolve_data_dir_falls_back_to_data_dir_when_unset` would otherwise rename it).
Concurrency: `cmd_open` resolves the dir and *then* spawns the daemon (`cli.rs:251,257`), so
two processes race. `fs::rename` is atomic on unix and the loser sees `ENOENT` — the loser
must treat "old dir already gone" as success, never as an error, or a normal `open` aborts.

Also in this cell: the **second data directory** rev 1 missed — `~/.cache/mdview/attach`
(`server.rs:2358,2370`). Renamed to `~/.cache/waggledance/attach` with **no** migration; it
is a cache, and the plan says so out loud rather than by omission.
Not touched: `herdr_config_dir()` (`herdr/socket.rs:40`) — confirmed to resolve
`$HOME/.config/herdr`, herdr's own namespace.
Also: `cli.rs:97` doc comment; the `.mdview` joins in `tests/e2e_stop_stale_lock.rs:42,47`
and `tests/e2e_open.rs:41`.
*New tests:* migrates once and carries `registry.db` across; skipped when the new dir exists;
no-op when the old dir is absent; the losing side of a concurrent rename succeeds; the suite
never touches the real home dir.

**W3 · MCP tool name, doctor stale-entry sweep, and the JSON-writer guard (D9).** *Test-first.*
`mcp.rs:59,85` tool name. Registration into `~/.claude.json`, `~/.codex/config.toml`, and the
Antigravity config writes the new name **and deletes the old `mdview` entry in the same write**.

Four defects found in the existing code, all inside this cell's blast radius:

- `doctor.rs:301-307` and `:357-364` **return early** the moment the new key exists — before
  any sweep. A config holding *both* names would report OK forever. The early return moves
  after the sweep.
- `doctor.rs:296-299` swallows a JSON parse failure into `json!({})` and then rewrites the
  whole file (`:338-340`). D9: refuse and leave unchanged, matching `doctor.rs:378-383`.
- `MDVIEW_START`/`MDVIEW_END` (`:483-484`) are matched by literal `text.find` (`:496-500`).
  Renaming the literal makes `write_agent_snippet` (`:517-533`) take the `else` branch and
  **append a second block**. This repo already carries the old markers — `AGENTS.md:185,213`,
  `CLAUDE.md:10,35` — so it would happen here first. Find the old marker, replace the block.
- `doctor.rs:600-604` `skill_path()` → `.claude/skills/mdview/SKILL.md`. Renaming it orphans
  `~/.claude/skills/mdview/SKILL.md`, which keeps telling agents to run a deleted binary and
  call a tool that no longer exists — the exact argument CONTEXT.md makes for the MCP entry.
  Doctor removes it.

Also: PATH probe `:90`, `current_exe_str()` fallback literal `:263-267`.
*New tests, per config format:* has-old → exactly one entry, new name; has-both → old gone;
has-neither → exactly one added; already-correct → idempotent; malformed JSON → refused and
byte-identical. Note: `serde_json` has no `preserve_order`, so `Map` is a `BTreeMap` and any
write alphabetically reorders the user's file. The idempotence test must assert on parsed
content, not bytes, and this reordering is recorded as known, not fixed here.

**W4 · Env vars, install scripts, release workflow (D7).**
`MDVIEW_HERDR_BINARY` (`supervisor.rs:33,38`) → `WAGGLEDANCE_HERDR_BINARY`, no old-name
fallback (D3). `MDVIEW_INSTALL_DIR`, `MDVIEW_VERSION` in both installers.
`install.sh:12-13,35,39` and `install.ps1:15-16,26`: `REPO`, `BIN`, the one-liner URLs, and
the D7 install dirs — `$HOME/.local/bin`, `%LOCALAPPDATA%\Programs\waggledance`.
`release.yml:70-75` asset `waggledance-<target>`.
*Verify:* `bash -n install.sh`; `pwsh` syntax parse if present, otherwise record its absence
and lean on review; the asset name in `release.yml` and the name `install.sh` downloads grep
to the same string.

**W5 · Web UI strings + browser storage fallback (D5).**
`app.js:15` `mdview-theme`, `:54,105,155,167` `mdview-folders-open`, and the inline reader at
`views.rs:28` → `waggledance-*` with the one-shot read-migrate-delete.
Rev 1 missed: `app.css:1198`, `app.js:379` body copy, and the `mdview:mermaid-done` event name
— **a pair**, `app.js:733` ↔ `views.rs:3759`, that must change together or the diagram-ready
handshake silently stops firing. Plus `views.rs:176` body copy.
**Flagged for veto at the gate:** the display brand `views.rs:23`
`<title>{title} · Bee Artifact</title>` and the `:229` aria-label become *Waggle Dance*.
This is a third name, not an `mdview` occurrence; it is here because the README now says
Waggle Dance and leaving "Bee Artifact" in the page title contradicts it.
*Verify:* `cargo test --workspace`; a test asserting the rendered title, and that no served
HTML/JS contains `mdview` outside the fallback lines.

**W6 · Daemon health handshake.** *New in rev 2 — a contract rev 1 missed entirely.*
`server.rs:755,763` answers health with `"app": "mdview"`; `daemon.rs:103` detects a live
daemon with `buf.contains("\"mdview\"")`. Rename one side only and daemon detection breaks
silently: auto-spawn stops recognising a running daemon. **One atomic cell, both files.**
Also here, since they sit in the same files: `server.rs:313,319,321,328` `mdview serving on …`
and `runtime.rs:109,115,128` auto-spawn messages, `:250` temp prefix `mdview-gate-…`.
*Verify:* `cargo test --workspace`; a test that a served `/health` body satisfies
`daemon.rs`'s own detection predicate — the two sides checked against each other, not against
a hardcoded string.

**W7 · Desktop crate.**
`git mv crates/mdview-desktop crates/waggledance-desktop`; package name, the
`waggledance-core` dep, `tauri.conf.json:3,5` → `dev.waggledance.app` (D6),
`main.rs:28,30,48,67,71,76,120,130,134,136,143` (`find_mdview`, `"Show mdview"`, title,
tooltip, `mdview.exe`, the spawned daemon binary name), `ui/index.html:5,22`,
`crates/mdview-desktop/README.md`, root `Cargo.toml:5` `exclude`, `.gitignore:56`, and the
**git-tracked** `crates/mdview-desktop/Cargo.lock`.
*Verify:* explicit, because this crate is **not** a workspace member —
`cargo build --manifest-path crates/waggledance-desktop/Cargo.toml`, and the regenerated
lockfile committed.

**W8 · Project marker (D8).** Small and separate because it is the one deliberate
two-name survivor: `cli.rs:317` `MARKERS` accepts `.waggledance.json` first, `.mdview.json`
still. *New test:* both names resolve the same project root.

## Slice 3 — the written record

**W9 · Docs, templates, paths, and the final sweep.**
`README.md`, `PRD.md`, `docs/usage.md`, `docs/specs/*` (9), `docs/knowledge/*`,
`docs/backlog.md`, `docs/mermaid-demo.md`, `AGENTS.md`, `CLAUDE.md`, and — missed in rev 1 —
`docs/distillery/` (5 files).
Renames, which a content grep can never catch: `docs/mdview-agents-template.md` and
`docs/mdview-skill-template.md`, the directory `plans/260715-1835-mdview-mvp/`, and the
report filename under `plans/reports/`, with every link updated.
*Verify:* `cargo test --workspace`; then a sweep over **content and paths**, excluding
`.git`, `.bee`, `target`, `docs/history`, returning only the seven survivors listed above —
`Cargo.lock` is **included** in the sweep this time, because rev 1 excluded it and a stale
lock would have passed its own completeness check.

## Ordering

W1 first. W2–W8 are disjoint by file and run in parallel after it; the review confirmed no
two of them touch the same file now that `server.rs` and `daemon.rs` are pinned to W6 alone.
W9 last, because it records the final names.

## Cost if the shape is wrong

W1 is one large mechanical commit; a crate rename has no green intermediate state, so
splitting it buys only a non-compiling middle, and redoing the `git mv` pair is cheap.
The expensive mistakes are W2, W3, and W6: a bad migration moves a user's registry somewhere
they cannot find it, a bad doctor write corrupts a config file shared with their other
tooling, and a half-renamed health handshake fails *silently* — no error, just a daemon
nobody can find. All three are test-first, and W2/W3 act only on their own key or their own
directory, never a whole-file rewrite.

## Not in this feature

Renaming `herdr` or its config. Fixing `serde_json` key reordering in `~/.claude.json`
(recorded, not fixed). Any behavior change beyond the rename, the D2 migration, and D9.

## What the review changed

- W2 redesigned: resolver-level, not daemon startup — the old guard could never fire.
- W6 created: the `server.rs` ↔ `daemon.rs` health handshake, a contract no rev 1 cell owned.
- W8 created: the `.mdview.json` marker (D8).
- Added to existing cells: `~/.cache/mdview/attach`, the doctor early-return and
  malformed-JSON defects, the marker-block duplication bug, the orphaned skill directory, the
  `mdview:mermaid-done` event pair, `app.css`, desktop `README.md` and its tracked
  `Cargo.lock`, `docs/distillery/`, and three path-shaped renames.
- Corrected: the occurrence count (1162/67, not 6228/77), `use mdview_core::` (26 lines, not
  28), `.gitignore:56` moved from W1 to W7.
- Ruled out: renaming the `short_link.rs` / `repository.rs` golden fixtures.
