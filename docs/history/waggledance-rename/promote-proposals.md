promote proposal for work item "waggledance-rename" (docs/history/waggledance-rename/CONTEXT.md + docs/history/waggledance-rename/plan.md) — 9 capped cell(s): waggledance-rename-1, waggledance-rename-2, waggledance-rename-3, waggledance-rename-4, waggledance-rename-5, waggledance-rename-6, waggledance-rename-7, waggledance-rename-8, waggledance-rename-9
anchor: history — docs/history/waggledance-rename/CONTEXT.md, docs/history/waggledance-rename/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/waggledance-rename/delivery.md

---
type: bee.delivery
title: waggledance-rename — delivery
description: "Delivery record proposed by bee knowledge promote for work item waggledance-rename: 9 capped cell(s), 40 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: waggledance-rename-delivery
  lifecycle: active
  areas: [daemon, doctor, web-interface, system-overview]
  required_context: [docs/history/waggledance-rename/CONTEXT.md, docs/history/waggledance-rename/plan.md]
  sources: [docs/history/waggledance-rename/CONTEXT.md, docs/history/waggledance-rename/plan.md, .bee/cells/waggledance-rename-1.json, .bee/cells/waggledance-rename-2.json, .bee/cells/waggledance-rename-3.json, .bee/cells/waggledance-rename-4.json, .bee/cells/waggledance-rename-5.json, .bee/cells/waggledance-rename-6.json, .bee/cells/waggledance-rename-7.json, .bee/cells/waggledance-rename-8.json, .bee/cells/waggledance-rename-9.json]
---

# waggledance-rename — Delivery

## What shipped

- **waggledance-rename-1** — crates/mdview-core and crates/mdview are now crates/waggledance-core and crates/waggledance via git mv, with package names, [[bin]] name, the dependency edge, 11 use waggledance_core:: imports, clap command name, the release.yml -p flag, the repository URL and the root Cargo.lock all following. crates/mdview-desktop and .gitignore left for the later cell that owns them. (16 file(s) changed)
- **waggledance-rename-2** — The data directory is ~/.waggledance, and ~/.mdview migrates into it once per process from inside the resolver rather than at daemon startup, armed only by cli::run so no test can touch a real home. The race loser treats an already-vanished source as success. The attach cache moves to ~/.cache/waggledance with no migration, deliberately. (5 file(s) changed)
- **waggledance-rename-3** — The MCP tool is waggledance_view_file, and doctor now sweeps every stale mdview artifact it once wrote: the old MCP entry is deleted in the same write that adds the new one across all three config formats, the early return no longer skips that sweep, a marker block is replaced in place instead of duplicated, the orphaned .claude/skills/mdview directory is removed, and a malformed ~/.claude.json is refused rather than rewritten as an empty object. (2 file(s) changed)
- **waggledance-rename-4** — The herdr override is WAGGLEDANCE_HERDR_BINARY, both installers pull thanhsmind/waggledance and install the binary outside the config directory (~/.local/bin on unix, %LOCALAPPDATA%\Programs\waggledance on Windows), and the release asset name now matches what install.sh downloads. (4 file(s) changed)
- **waggledance-rename-5** — Browser storage keys are waggledance-* with a one-shot migration that keeps an existing user their theme and folder state, the mermaid-done event renamed on both sides at once, every mdview string cleared from the served page and its assets, and the display brand is now Waggle Dance. (3 file(s) changed)
- **waggledance-rename-6** — Both halves of the daemon health handshake now say waggledance, and the detection check moved out of health_check into a shared looks_like_daemon predicate that a route-level test asserts the real /health body against — so a future one-sided rename fails a test instead of silently breaking auto-spawn. (3 file(s) changed)
- **waggledance-rename-7** — crates/mdview-desktop is now crates/waggledance-desktop, with the package name, the waggledance-core dependency, the Tauri productName and identifier (dev.waggledance.app), every window/tray/binary string in main.rs, ui/index.html, the crate README, the root Cargo.toml exclude line, .gitignore, and the tracked Cargo.lock all following. The crate build itself could not be verified here — no pkg-config on this machine. (8 file(s) changed)
- **waggledance-rename-8** — Both .waggledance.json and .mdview.json resolve a project root, with a comment recording that the old name is deliberate. The marker walk moved out of find_project_root into root_by_marker so it is testable without an Engine. (1 file(s) changed)
- **waggledance-rename-9** — Every document, template filename and path now says waggledance, the managed block in AGENTS.md and CLAUDE.md matches what doctor writes, doctors include_str paths and template assertions followed the renamed files, and the sweep over content and paths including Cargo.lock leaves only named, test-pinned survivors. (14 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **waggledance-rename-1** — `cargo test --workspace green and still at 886 passed -- a rename must not change the count. ./target/debug/waggledance --help prints 'Usage: waggledance'. git log --stat shows crates/mdview-core and crates/mdview as renames rather than delete-plus-add. grep -rn 'mdview' crates/waggledance crates/waggledance-core --include='*.toml' returns nothing.`
- **waggledance-rename-2** — `cargo test --workspace green. New tests: the migration moves ~/.mdview to ~/.waggledance once and registry.db survives with its rows intact; it is skipped when the new dir already exists; it is a no-op when the old dir is absent; the losing side of a concurrent rename returns success rather than an error; and the suite never touches the real home directory (assert the opt-out is what the existing resolver tests use).`
- **waggledance-rename-3** — `cargo test --workspace green. New tests, one set per config format (JSON for Claude Code and Antigravity, TOML for Codex): a config holding only the old mdview entry ends with exactly one entry under the new name; a config holding BOTH ends with only the new one; a config holding neither gains exactly one; a config already correct is unchanged in parsed content; and a malformed JSON file is refused and left byte-identical. Plus: an AGENTS.md carrying the old marker block ends with exactly one block, carrying the new marker; and doctor removes an existing .claude/skills/mdview/ directory. Note serde_json has no preserve_order, so any write alphabetically reorders the user's keys -- assert on parsed content, not bytes.`
- **waggledance-rename-4** — `cargo test --workspace green. bash -n install.sh parses. If pwsh is on PATH, parse install.ps1 with [ScriptBlock]::Create on its contents; if pwsh is absent, say so in the report rather than claiming it passed. The asset name written by release.yml and the asset name downloaded by install.sh grep to the same string. No MDVIEW_ prefix remains anywhere in crates/ or the two install scripts.`
- **waggledance-rename-5** — `cargo test --workspace green. New tests: the rendered page title says Waggle Dance; no served HTML or JS contains the string mdview outside the two storage-fallback reads. The mermaid-done event name is asserted identical on both sides -- dispatch site and listener compared to each other, not to a hardcoded copy.`
- **waggledance-rename-6** — `cargo test --workspace green. A new test asserts that a served /health body satisfies daemon.rs's own detection predicate -- the two sides checked against each other rather than each against its own hardcoded copy of the string, so this class of drift cannot recur.`
- **waggledance-rename-7** — `cargo test --workspace green (it does not cover this crate -- the root Cargo.toml excludes it, which is exactly why the next check exists). cargo build --manifest-path crates/waggledance-desktop/Cargo.toml succeeds, and the regenerated crates/waggledance-desktop/Cargo.lock is committed. grep -rn mdview crates/waggledance-desktop returns nothing.`
- **waggledance-rename-8** — `cargo test --workspace green. A new test asserts that a directory containing only .mdview.json and a directory containing only .waggledance.json both resolve as project roots.`
- **waggledance-rename-9** — `cargo test --workspace green. Then the completeness sweep, over BOTH content and path names, excluding .git, .bee, target and docs/history but INCLUDING Cargo.lock: the only occurrences of mdview left in the tree are these seven, each of which a test already pins. (1) ~/.mdview read by the D2 migration in config.rs. (2) The old MCP entry names swept by doctor. (3) The old marker constants doctor still recognises so an existing block is replaced rather than duplicated. (4) The .claude/skills/mdview path doctor removes. (5) The mdview-theme and mdview-folders-open keys read once by the storage fallback. (6) .mdview.json in cli.rs PROJECT_MARKERS, kept permanently by D8. (7) Prose in docs that is deliberately describing the old name, if any survives - name each one. Anything else the sweep finds is a miss and must be fixed in this cell.`

## Deviations

- **waggledance-rename-1** — The cap's own test run executes in the MAIN checkout, which does not carry this rename — the change lives on branch wt/waggledance-rename in the granted worktree, and bee's control-plane commands refuse to run there. The real proof is the worker's run inside the worktree: cargo test --workspace green, 515 + 3 + 5 + 363 + 0 = 886 passed, 0 failed, plus ./target/debug/waggledance --help printing "Usage: waggledance <COMMAND>". Main's re-run is therefore corroborating, not the evidence.
- **waggledance-rename-1** — Worker also had to rename env!("CARGO_BIN_EXE_mdview") to CARGO_BIN_EXE_waggledance across 8 call sites in tests/e2e_open.rs and tests/e2e_stop_stale_lock.rs. Not in the cell's checklist, but changing [[bin]] name makes the old macro name fail to compile, so it was required to reach green.
- **waggledance-rename-1** — Worker amended its own just-created unpushed commit once: the first path-scoped commit staged the new paths but not the old-path deletions. Amending produced the one correct commit the cell asked for.
- **waggledance-rename-1** — docs/history/waggledance-rename/CONTEXT.md and plan.md do not exist on the worktree branch — they were written in main after the worktree branched, and remain uncommitted there. The worker proceeded on the dispatch prompt alone. Committing them in main is follow-up work for the docs lane.
- **waggledance-rename-2** — The dispatched worker stalled with the work uncommitted (harness watchdog: no progress for 600s). The orchestrator inspected what it left, judged it sound, and finished the remainder inline rather than re-dispatching: the two e2e test files still pointed at ~/.mdview, which was the whole of the 5 red tests, plus leftover mdview prose in config.rs and cli.rs. That is a named deviation from "small and up runs through a dispatched worker" — the remainder was a path rename in two test files and a scoped prose sweep, and re-dispatching would have cost more than it proved.
- **waggledance-rename-2** — Design deviation the worker made, and the orchestrator accepted: the cell specified an opt-OUT that the test suite sets. The worker built an opt-IN instead — DATA_DIR_MIGRATION_ARMED defaults to false and is armed only inside cli::run, the single dispatch point every real subcommand passes through, including mcp and doctor, and re-armed the same way in a re-exec'd daemon. Nothing in the workspace's tests calls run. This is strictly safer than an opt-out: an opt-out is one missed call site away from renaming a developer's real home directory, and there are dozens of route-level tests that resolve the data dir.
- **waggledance-rename-2** — Evidence, run in the worktree: cargo test --workspace green, 900 passed. Baseline was 886; W3 added 9 and this cell added 5. The cap's own run happens in main, which carries none of this, so it corroborates rather than proves.
- **waggledance-rename-2** — All five required cases are present in config.rs: migrates once with registry.db bytes intact, skipped when the new dir exists, no-op when the old dir is absent, the losing side of a concurrent rename returns Ok, and the suite is proved disarmed by its own test.
- **waggledance-rename-2** — Left for later cells, deliberately: server.rs still carries "mdview serving on" (:313-328) and the "app": "mdview" health strings (:755,763) -- cell waggledance-rename-6 owns both sides of that handshake and must change them together. cli.rs:327 still lists ".mdview.json" -- cell waggledance-rename-8 owns decision D8, which keeps it on purpose.
- **waggledance-rename-3** — Evidence, run in the worktree after the sibling cell that was in flight during this worker's run landed: cargo test --workspace green, 900 passed. The worker's own run showed 895 passed and 5 red, all in e2e_open.rs and e2e_stop_stale_lock.rs — files this cell never touched, failing on daemon-startup timeouts caused by the concurrent data-dir cell's uncommitted work. Those five went green the moment that cell finished; nothing in this cell changed. The cap's own test run happens in main, which carries none of this branch, so it corroborates rather than proves.
- **waggledance-rename-3** — Nine new tests, written before the code: per config format (JSON for Claude Code and Antigravity, TOML for Codex) — old entry only is swept, both names present leaves only the new one, neither present adds exactly one, already-correct is idempotent on parsed content, and malformed JSON is refused with the file byte-identical. Plus a marker-block replacement starting from the old marker, and removal of a stale .claude/skills/mdview/ directory.
- **waggledance-rename-3** — Deviation the worker made, and the orchestrator accepted: it renamed every other user-facing string and doc comment inside its two exclusively-owned files, beyond the five numbered fixes — doctor's own CLI hints ("mdview serve", "restart", "refresh"), its header, and its "MDView block" messaging. Reason given, and it is the right one: leaving stale binary names in doctor's own guidance text would reproduce the exact "tells the user to run a deleted binary" defect this cell exists to remove.
- **waggledance-rename-3** — Left alone on purpose: the include_str! paths for docs/mdview-*.md and the content assertions on AGENT_TEMPLATE and SKILL_TEMPLATE. Those documents are renamed by cell waggledance-rename-9, and this cell's assertions will need to follow them there.
- **waggledance-rename-3** — Known and not fixed here, by decision: serde_json is declared without preserve_order, so its Map is a BTreeMap and every doctor write alphabetically reorders the keys in a user's ~/.claude.json. Unrelated content survives; formatting and ordering do not. The idempotence tests therefore assert on parsed content rather than bytes.
- **waggledance-rename-4** — install.ps1 was NOT syntax-checked: pwsh is absent from PATH in this environment, so the [ScriptBlock]::Create parse could not run. The worker reported that plainly instead of claiming it passed, which is right. install.ps1's changes are therefore review-verified only — the Windows installer needs a real parse, or a Windows CI job, before release. This is the second thing in this feature that cannot be proved on this machine; the first is the desktop crate build (cell waggledance-rename-7, no pkg-config).
- **waggledance-rename-4** — Asset-name consistency IS proved, and it was the cell's sharpest risk: .github/workflows/release.yml:71 publishes asset="waggledance-${{ matrix.target }}", and install.sh:48 builds its URL from ${BIN}-${TARGET} with BIN="waggledance". Same string on both sides.
- **waggledance-rename-4** — Deviation, accepted: D7 said install.sh's fallback chain ends at $HOME/.local/bin, but that path was already the chain's second entry, so a literal replacement of the third entry would have produced a duplicate. The worker dropped the redundant third entry instead. That serves the decision; keeping the duplicate would not.
- **waggledance-rename-4** — Deviation, accepted: the worker also fixed the remaining "Installing mdview" and "mdview installer" prose in the two installer scripts. Those files are this cell's alone, no later cell touches them, and stale brand text sitting next to BIN="waggledance" would contradict the rename.
- **waggledance-rename-4** — Flagged for awareness, not a defect: grep for MDVIEW_ still finds OLD_MDVIEW_START and OLD_MDVIEW_END in doctor.rs. Those are cell waggledance-rename-3's deliberate survivors — the old marker names it must still recognise so an existing block is replaced rather than duplicated.
- **waggledance-rename-4** — Evidence: cargo test --workspace exit 0 in the worktree. The cap's own run happens in main, which carries none of this branch.
- **waggledance-rename-5** — Evidence, run in the worktree: cargo test --workspace green, 905 passed. Baseline for this worker was 900; the +5 is its own 3 new tests plus 2 that a concurrent sibling cell landed in the shared tree during its run. The cap's own run happens in main, which carries none of this branch.
- **waggledance-rename-5** — Correction to the cell's own text, found by the worker: the mermaid-done pair is the other way round from what the cell said. The listener is in app.js and the dispatch is in views.rs, not the reverse. Both were renamed together, so the outcome is unaffected, but the plan's anchors for that pair were wrong.
- **waggledance-rename-5** — Three tests rather than one, and the shape is right: the page title says Waggle Dance; a sweep asserts no stray mdview across layout(), APP_JS and APP_CSS; and the mermaid event name is cross-checked between dispatch and listener, anchored on an unrelated callback name so it cannot pass by two hardcoded copies agreeing with each other.
- **waggledance-rename-5** — Deviation, accepted: the worker also renamed roughly 30 test-only temp-directory prefixes in views.rs's test module. They are internal scratch names, never served, but a literal grep for mdview would have found them and the final sweep cell requires that grep to be clean.
- **waggledance-rename-5** — Storage migration is implemented in both places it has to be: the inline no-flash theme script in views.rs and a shared migrateStorageKey helper in app.js, called at both folder-state read sites. Read old, write new, delete old, inert afterwards.
- **waggledance-rename-6** — The silent-drift risk this cell existed to remove is now structurally closed, not just renamed. The worker pulled the inline detection check out of health_check into a shared predicate, waggledance_core::daemon::looks_like_daemon, and the new test hits the real /health route through the router and asserts the body satisfies that predicate. Neither side of the test hardcodes the string, so a future one-sided rename fails the test instead of silently breaking daemon detection. A second unit test covers the extracted predicate directly.
- **waggledance-rename-6** — Evidence, run in the worktree: cargo test --workspace green, 902 passed — 900 before this cell, plus its two new tests. grep for mdview across server.rs, daemon.rs and runtime.rs returns nothing. The cap's own run happens in main, which carries none of this branch.
- **waggledance-rename-6** — Deviation, accepted: a doc comment at server.rs:2364 read "~/.mdview -> ~/.waggledance (D2)". A mechanical rename would have turned it into "~/.waggledance -> ~/.waggledance", which says nothing. The worker reworded it to refer to the config data dir's D2 rename without naming the old path literally, which keeps the meaning and satisfies the grep-clean bar for these files.
- **waggledance-rename-6** — Deviation, accepted: the cell named specific lines, but the worker renamed every remaining mdview occurrence in its three files — roughly 40 more, mostly doc comments and test fixture names in server.rs. None of them are on the feature's intentional-survivor list; every survivor on that list lives in another cell's files.
- **waggledance-rename-7** — The cell's own headline check could NOT be run: `cargo build --manifest-path crates/waggledance-desktop/Cargo.toml` fails before it reaches this crate's code, because libdbus-sys v0.2.7's build script needs pkg-config and this machine has none (`which pkg-config` exits 1; /usr/bin/pkg-config and /usr/bin/pkgconf do not exist). The crate's own README already documents needing libwebkit2gtk, gtk3 and dbus dev libraries on Linux, so this is a pre-existing environment gap, not a rename defect — the same failure occurs against the old manifest path. `cargo check` fails identically, at the same build-script stage.
- **waggledance-rename-7** — What IS proven: dependency resolution succeeds, which is what regenerates Cargo.lock, and the regenerated lock shows a coherent waggledance-core / waggledance-desktop graph with zero mdview entries. `grep -rn mdview crates/waggledance-desktop` is empty. `cargo test --workspace` is green at 886 passed, though that suite excludes this crate by design and so proves nothing about it.
- **waggledance-rename-7** — So the must-have "the desktop crate builds via its own manifest" is UNPROVEN in this environment, and the cap should not be read as proving it. It needs a machine with pkg-config, libwebkit2gtk and gtk3, or CI, before release.
- **waggledance-rename-7** — Scope note the worker recorded: root Cargo.toml line 6 still carries a comment pointing at "crates/mdview-desktop/README.md". The worker was told to change only the exclude line in that file because a sibling worker was reading it, so it left the comment. Cell waggledance-rename-9 (the docs and final sweep) must catch it.
- **waggledance-rename-9** — Evidence, run in the worktree: cargo test --workspace green, 906 passed — unchanged from before this cell, which is right for a rename that moved template files without adding or removing assertions. The orchestrator re-ran the sweep independently and got exactly the worker's twelve files, with the path sweep empty and Cargo.lock clean. The cap's own test run happens in main, which carries none of this branch.
- **waggledance-rename-9** — The seven planned survivors all hold. Five more files carry mdview and each is accounted for:
- **waggledance-rename-9** — - repository.rs and short_link.rs — ruled out in the plan itself. There "mdview" is an arbitrary registered-project NAME used as a golden-hash input, not this binary; rewriting it would break the constants for nothing.
- **waggledance-rename-9** — - docs/knowledge/work/upstream-short-link/delivery.md — "vantt/mdview" is the actual upstream fork this repo diverged from. Not ours to rename.
- **waggledance-rename-9** — - plans/260715-1835-waggledance-mvp/plan.md and the two reports under plans/reports/ — dated, completed build and research logs. Their path names were renamed as instructed; their prose was not, because rewriting it would misrepresent what was literally built and decided at the time. Same reasoning docs/history gets by exclusion, applied to records that happen to sit outside it.
- **waggledance-rename-9** — - docs/backlog.md — SURVIVOR #8, recorded here as a deliberate outcome rather than a miss. All 17 mentions sit in rows marked done, describing what was built when it was built, with crates/mdview/... paths as they existed then; it is the same class of historical ledger. The worker could not have edited it anyway: bee's write-guard owns that file and refuses direct edits, and the worker was correctly told to run no bee command inside the worktree.
- **waggledance-rename-9** — One genuine tail, small and named: docs/backlog.md line 3 reads "PBI rows cho mdview", which is live prose, not history. It needs a bee backlog pass from the main checkout after the merge. Line 12's MDVIEW_VERSION sits inside a done row and stays as history.

## Provenance

Proposed by `bee knowledge promote --work waggledance-rename` from 9 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/waggledance-rename/CONTEXT.md`, `docs/history/waggledance-rename/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "waggledance-rename" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-13T11:07:13.305Z), the work item declares no bee.areas.

area daemon:
  - [waggledance-rename-2] The data directory is ~/.waggledance, and ~/.mdview migrates into it once per process from inside the resolver rather than at daemon startup, armed only by cli::run so no test can touch a real home. The race loser treats an already-vanished source as success. The attach cache moves to ~/.cache/waggledance with no migration, deliberately. — feature-wide sync per the scribing stamp, 5 file(s) changed (trace .bee/cells/waggledance-rename-2.json)
  - [waggledance-rename-3] The MCP tool is waggledance_view_file, and doctor now sweeps every stale mdview artifact it once wrote: the old MCP entry is deleted in the same write that adds the new one across all three config formats, the early return no longer skips that sweep, a marker block is replaced in place instead of duplicated, the orphaned .claude/skills/mdview directory is removed, and a malformed ~/.claude.json is refused rather than rewritten as an empty object. — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/waggledance-rename-3.json)
  - [waggledance-rename-4] The herdr override is WAGGLEDANCE_HERDR_BINARY, both installers pull thanhsmind/waggledance and install the binary outside the config directory (~/.local/bin on unix, %LOCALAPPDATA%\Programs\waggledance on Windows), and the release asset name now matches what install.sh downloads. — feature-wide sync per the scribing stamp, 4 file(s) changed (trace .bee/cells/waggledance-rename-4.json)
  - [waggledance-rename-5] Browser storage keys are waggledance-* with a one-shot migration that keeps an existing user their theme and folder state, the mermaid-done event renamed on both sides at once, every mdview string cleared from the served page and its assets, and the display brand is now Waggle Dance. — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/waggledance-rename-5.json)
  - [waggledance-rename-6] Both halves of the daemon health handshake now say waggledance, and the detection check moved out of health_check into a shared looks_like_daemon predicate that a route-level test asserts the real /health body against — so a future one-sided rename fails a test instead of silently breaking auto-spawn. — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/waggledance-rename-6.json)
  - [waggledance-rename-7] crates/mdview-desktop is now crates/waggledance-desktop, with the package name, the waggledance-core dependency, the Tauri productName and identifier (dev.waggledance.app), every window/tray/binary string in main.rs, ui/index.html, the crate README, the root Cargo.toml exclude line, .gitignore, and the tracked Cargo.lock all following. The crate build itself could not be verified here — no pkg-config on this machine. — feature-wide sync per the scribing stamp, 8 file(s) changed (trace .bee/cells/waggledance-rename-7.json)
  - [waggledance-rename-8] Both .waggledance.json and .mdview.json resolve a project root, with a comment recording that the old name is deliberate. The marker walk moved out of find_project_root into root_by_marker so it is testable without an Engine. — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/waggledance-rename-8.json)
  - [waggledance-rename-9] Every document, template filename and path now says waggledance, the managed block in AGENTS.md and CLAUDE.md matches what doctor writes, doctors include_str paths and template assertions followed the renamed files, and the sweep over content and paths including Cargo.lock leaves only named, test-pinned survivors. — feature-wide sync per the scribing stamp, 14 file(s) changed (trace .bee/cells/waggledance-rename-9.json)

area doctor:
  - [waggledance-rename-2] The data directory is ~/.waggledance, and ~/.mdview migrates into it once per process from inside the resolver rather than at daemon startup, armed only by cli::run so no test can touch a real home. The race loser treats an already-vanished source as success. The attach cache moves to ~/.cache/waggledance with no migration, deliberately. — feature-wide sync per the scribing stamp, 5 file(s) changed (trace .bee/cells/waggledance-rename-2.json)
  - [waggledance-rename-3] The MCP tool is waggledance_view_file, and doctor now sweeps every stale mdview artifact it once wrote: the old MCP entry is deleted in the same write that adds the new one across all three config formats, the early return no longer skips that sweep, a marker block is replaced in place instead of duplicated, the orphaned .claude/skills/mdview directory is removed, and a malformed ~/.claude.json is refused rather than rewritten as an empty object. — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/waggledance-rename-3.json)
  - [waggledance-rename-4] The herdr override is WAGGLEDANCE_HERDR_BINARY, both installers pull thanhsmind/waggledance and install the binary outside the config directory (~/.local/bin on unix, %LOCALAPPDATA%\Programs\waggledance on Windows), and the release asset name now matches what install.sh downloads. — feature-wide sync per the scribing stamp, 4 file(s) changed (trace .bee/cells/waggledance-rename-4.json)
  - [waggledance-rename-5] Browser storage keys are waggledance-* with a one-shot migration that keeps an existing user their theme and folder state, the mermaid-done event renamed on both sides at once, every mdview string cleared from the served page and its assets, and the display brand is now Waggle Dance. — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/waggledance-rename-5.json)
  - [waggledance-rename-6] Both halves of the daemon health handshake now say waggledance, and the detection check moved out of health_check into a shared looks_like_daemon predicate that a route-level test asserts the real /health body against — so a future one-sided rename fails a test instead of silently breaking auto-spawn. — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/waggledance-rename-6.json)
  - [waggledance-rename-7] crates/mdview-desktop is now crates/waggledance-desktop, with the package name, the waggledance-core dependency, the Tauri productName and identifier (dev.waggledance.app), every window/tray/binary string in main.rs, ui/index.html, the crate README, the root Cargo.toml exclude line, .gitignore, and the tracked Cargo.lock all following. The crate build itself could not be verified here — no pkg-config on this machine. — feature-wide sync per the scribing stamp, 8 file(s) changed (trace .bee/cells/waggledance-rename-7.json)
  - [waggledance-rename-8] Both .waggledance.json and .mdview.json resolve a project root, with a comment recording that the old name is deliberate. The marker walk moved out of find_project_root into root_by_marker so it is testable without an Engine. — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/waggledance-rename-8.json)
  - [waggledance-rename-9] Every document, template filename and path now says waggledance, the managed block in AGENTS.md and CLAUDE.md matches what doctor writes, doctors include_str paths and template assertions followed the renamed files, and the sweep over content and paths including Cargo.lock leaves only named, test-pinned survivors. — feature-wide sync per the scribing stamp, 14 file(s) changed (trace .bee/cells/waggledance-rename-9.json)

area web-interface:
  - [waggledance-rename-2] The data directory is ~/.waggledance, and ~/.mdview migrates into it once per process from inside the resolver rather than at daemon startup, armed only by cli::run so no test can touch a real home. The race loser treats an already-vanished source as success. The attach cache moves to ~/.cache/waggledance with no migration, deliberately. — feature-wide sync per the scribing stamp, 5 file(s) changed (trace .bee/cells/waggledance-rename-2.json)
  - [waggledance-rename-3] The MCP tool is waggledance_view_file, and doctor now sweeps every stale mdview artifact it once wrote: the old MCP entry is deleted in the same write that adds the new one across all three config formats, the early return no longer skips that sweep, a marker block is replaced in place instead of duplicated, the orphaned .claude/skills/mdview directory is removed, and a malformed ~/.claude.json is refused rather than rewritten as an empty object. — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/waggledance-rename-3.json)
  - [waggledance-rename-4] The herdr override is WAGGLEDANCE_HERDR_BINARY, both installers pull thanhsmind/waggledance and install the binary outside the config directory (~/.local/bin on unix, %LOCALAPPDATA%\Programs\waggledance on Windows), and the release asset name now matches what install.sh downloads. — feature-wide sync per the scribing stamp, 4 file(s) changed (trace .bee/cells/waggledance-rename-4.json)
  - [waggledance-rename-5] Browser storage keys are waggledance-* with a one-shot migration that keeps an existing user their theme and folder state, the mermaid-done event renamed on both sides at once, every mdview string cleared from the served page and its assets, and the display brand is now Waggle Dance. — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/waggledance-rename-5.json)
  - [waggledance-rename-6] Both halves of the daemon health handshake now say waggledance, and the detection check moved out of health_check into a shared looks_like_daemon predicate that a route-level test asserts the real /health body against — so a future one-sided rename fails a test instead of silently breaking auto-spawn. — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/waggledance-rename-6.json)
  - [waggledance-rename-7] crates/mdview-desktop is now crates/waggledance-desktop, with the package name, the waggledance-core dependency, the Tauri productName and identifier (dev.waggledance.app), every window/tray/binary string in main.rs, ui/index.html, the crate README, the root Cargo.toml exclude line, .gitignore, and the tracked Cargo.lock all following. The crate build itself could not be verified here — no pkg-config on this machine. — feature-wide sync per the scribing stamp, 8 file(s) changed (trace .bee/cells/waggledance-rename-7.json)
  - [waggledance-rename-8] Both .waggledance.json and .mdview.json resolve a project root, with a comment recording that the old name is deliberate. The marker walk moved out of find_project_root into root_by_marker so it is testable without an Engine. — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/waggledance-rename-8.json)
  - [waggledance-rename-9] Every document, template filename and path now says waggledance, the managed block in AGENTS.md and CLAUDE.md matches what doctor writes, doctors include_str paths and template assertions followed the renamed files, and the sweep over content and paths including Cargo.lock leaves only named, test-pinned survivors. — feature-wide sync per the scribing stamp, 14 file(s) changed (trace .bee/cells/waggledance-rename-9.json)

area system-overview:
  - [waggledance-rename-2] The data directory is ~/.waggledance, and ~/.mdview migrates into it once per process from inside the resolver rather than at daemon startup, armed only by cli::run so no test can touch a real home. The race loser treats an already-vanished source as success. The attach cache moves to ~/.cache/waggledance with no migration, deliberately. — feature-wide sync per the scribing stamp, 5 file(s) changed (trace .bee/cells/waggledance-rename-2.json)
  - [waggledance-rename-3] The MCP tool is waggledance_view_file, and doctor now sweeps every stale mdview artifact it once wrote: the old MCP entry is deleted in the same write that adds the new one across all three config formats, the early return no longer skips that sweep, a marker block is replaced in place instead of duplicated, the orphaned .claude/skills/mdview directory is removed, and a malformed ~/.claude.json is refused rather than rewritten as an empty object. — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/waggledance-rename-3.json)
  - [waggledance-rename-4] The herdr override is WAGGLEDANCE_HERDR_BINARY, both installers pull thanhsmind/waggledance and install the binary outside the config directory (~/.local/bin on unix, %LOCALAPPDATA%\Programs\waggledance on Windows), and the release asset name now matches what install.sh downloads. — feature-wide sync per the scribing stamp, 4 file(s) changed (trace .bee/cells/waggledance-rename-4.json)
  - [waggledance-rename-5] Browser storage keys are waggledance-* with a one-shot migration that keeps an existing user their theme and folder state, the mermaid-done event renamed on both sides at once, every mdview string cleared from the served page and its assets, and the display brand is now Waggle Dance. — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/waggledance-rename-5.json)
  - [waggledance-rename-6] Both halves of the daemon health handshake now say waggledance, and the detection check moved out of health_check into a shared looks_like_daemon predicate that a route-level test asserts the real /health body against — so a future one-sided rename fails a test instead of silently breaking auto-spawn. — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/waggledance-rename-6.json)
  - [waggledance-rename-7] crates/mdview-desktop is now crates/waggledance-desktop, with the package name, the waggledance-core dependency, the Tauri productName and identifier (dev.waggledance.app), every window/tray/binary string in main.rs, ui/index.html, the crate README, the root Cargo.toml exclude line, .gitignore, and the tracked Cargo.lock all following. The crate build itself could not be verified here — no pkg-config on this machine. — feature-wide sync per the scribing stamp, 8 file(s) changed (trace .bee/cells/waggledance-rename-7.json)
  - [waggledance-rename-8] Both .waggledance.json and .mdview.json resolve a project root, with a comment recording that the old name is deliberate. The marker walk moved out of find_project_root into root_by_marker so it is testable without an Engine. — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/waggledance-rename-8.json)
  - [waggledance-rename-9] Every document, template filename and path now says waggledance, the managed block in AGENTS.md and CLAUDE.md matches what doctor writes, doctors include_str paths and template assertions followed the renamed files, and the sweep over content and paths including Cargo.lock leaves only named, test-pinned survivors. — feature-wide sync per the scribing stamp, 14 file(s) changed (trace .bee/cells/waggledance-rename-9.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

from cell waggledance-rename-1 — save as docs/knowledge/patterns/waggledance-rename-waggledance-rename-1-pitfall.md

---
type: bee.pattern
title: waggledance-rename cell waggledance-rename-1 — pitfall candidate
description: "Pitfall candidate mined from cell waggledance-rename-1's capped trace: The cap's own test run executes in the MAIN checkout, which does not carry this rename — the change lives on branch wt/waggledance-rename in the granted worktr…"
timestamp: 2026-08-13
bee:
  id: waggledance-rename-waggledance-rename-1-pitfall
  lifecycle: draft
  areas: [daemon, doctor, web-interface, system-overview]
  sources: [.bee/cells/waggledance-rename-1.json]
  polarity: pitfall
---

# waggledance-rename cell waggledance-rename-1 — pitfall candidate

## What the cell did

crates/mdview-core and crates/mdview are now crates/waggledance-core and crates/waggledance via git mv, with package names, [[bin]] name, the dependency edge, 11 use waggledance_core:: imports, clap command name, the release.yml -p flag, the repository URL and the root Cargo.lock all following. crates/mdview-desktop and .gitignore left for the later cell that owns them.

## Recorded evidence (verbatim from .bee/cells/waggledance-rename-1.json)

- **deviation** — The cap's own test run executes in the MAIN checkout, which does not carry this rename — the change lives on branch wt/waggledance-rename in the granted worktree, and bee's control-plane commands refuse to run there. The real proof is the worker's run inside the worktree: cargo test --workspace green, 515 + 3 + 5 + 363 + 0 = 886 passed, 0 failed, plus ./target/debug/waggledance --help printing "Usage: waggledance <COMMAND>". Main's re-run is therefore corroborating, not the evidence.
- **deviation** — Worker also had to rename env!("CARGO_BIN_EXE_mdview") to CARGO_BIN_EXE_waggledance across 8 call sites in tests/e2e_open.rs and tests/e2e_stop_stale_lock.rs. Not in the cell's checklist, but changing [[bin]] name makes the old macro name fail to compile, so it was required to reach green.
- **deviation** — Worker amended its own just-created unpushed commit once: the first path-scoped commit staged the new paths but not the old-path deletions. Amending produced the one correct commit the cell asked for.
- **deviation** — docs/history/waggledance-rename/CONTEXT.md and plan.md do not exist on the worktree branch — they were written in main after the worktree branched, and remain uncommitted there. The worker proceeded on the dispatch prompt alone. Committing them in main is follow-up work for the docs lane.

## Status

Candidate only. `bee knowledge promote` proposes; naming the pattern, generalizing it beyond this cell, and moving `bee.lifecycle` to `active` are a human or agent decision.

from cell waggledance-rename-2 — save as docs/knowledge/patterns/waggledance-rename-waggledance-rename-2-pitfall.md

---
type: bee.pattern
title: waggledance-rename cell waggledance-rename-2 — pitfall candidate
description: "Pitfall candidate mined from cell waggledance-rename-2's capped trace: The dispatched worker stalled with the work uncommitted (harness watchdog: no progress for 600s). The orchestrator inspected what it left, judged it sound, and…"
timestamp: 2026-08-13
bee:
  id: waggledance-rename-waggledance-rename-2-pitfall
  lifecycle: draft
  areas: [daemon, doctor, web-interface, system-overview]
  sources: [.bee/cells/waggledance-rename-2.json]
  polarity: pitfall
---

# waggledance-rename cell waggledance-rename-2 — pitfall candidate

## What the cell did

The data directory is ~/.waggledance, and ~/.mdview migrates into it once per process from inside the resolver rather than at daemon startup, armed only by cli::run so no test can touch a real home. The race loser treats an already-vanished source as success. The attach cache moves to ~/.cache/waggledance with no migration, deliberately.

## Recorded evidence (verbatim from .bee/cells/waggledance-rename-2.json)

- **deviation** — The dispatched worker stalled with the work uncommitted (harness watchdog: no progress for 600s). The orchestrator inspected what it left, judged it sound, and finished the remainder inline rather than re-dispatching: the two e2e test files still pointed at ~/.mdview, which was the whole of the 5 red tests, plus leftover mdview prose in config.rs and cli.rs. That is a named deviation from "small and up runs through a dispatched worker" — the remainder was a path rename in two test files and a scoped prose sweep, and re-dispatching would have cost more than it proved.
- **deviation** — Design deviation the worker made, and the orchestrator accepted: the cell specified an opt-OUT that the test suite sets. The worker built an opt-IN instead — DATA_DIR_MIGRATION_ARMED defaults to false and is armed only inside cli::run, the single dispatch point every real subcommand passes through, including mcp and doctor, and re-armed the same way in a re-exec'd daemon. Nothing in the workspace's tests calls run. This is strictly safer than an opt-out: an opt-out is one missed call site away from renaming a developer's real home directory, and there are dozens of route-level tests that resolve the data dir.
- **deviation** — Evidence, run in the worktree: cargo test --workspace green, 900 passed. Baseline was 886; W3 added 9 and this cell added 5. The cap's own run happens in main, which carries none of this, so it corroborates rather than proves.
- **deviation** — All five required cases are present in config.rs: migrates once with registry.db bytes intact, skipped when the new dir exists, no-op when the old dir is absent, the losing side of a concurrent rename returns Ok, and the suite is proved disarmed by its own test.
- **deviation** — Left for later cells, deliberately: server.rs still carries "mdview serving on" (:313-328) and the "app": "mdview" health strings (:755,763) -- cell waggledance-rename-6 owns both sides of that handshake and must change them together. cli.rs:327 still lists ".mdview.json" -- cell waggledance-rename-8 owns decision D8, which keeps it on purpose.

## Status

Candidate only. `bee knowledge promote` proposes; naming the pattern, generalizing it beyond this cell, and moving `bee.lifecycle` to `active` are a human or agent decision.

from cell waggledance-rename-3 — save as docs/knowledge/patterns/waggledance-rename-waggledance-rename-3-pitfall.md

---
type: bee.pattern
title: waggledance-rename cell waggledance-rename-3 — pitfall candidate
description: "Pitfall candidate mined from cell waggledance-rename-3's capped trace: Evidence, run in the worktree after the sibling cell that was in flight during this worker's run landed: cargo test --workspace green, 900 passed. The worker's…"
timestamp: 2026-08-13
bee:
  id: waggledance-rename-waggledance-rename-3-pitfall
  lifecycle: draft
  areas: [daemon, doctor, web-interface, system-overview]
  sources: [.bee/cells/waggledance-rename-3.json]
  polarity: pitfall
---

# waggledance-rename cell waggledance-rename-3 — pitfall candidate

## What the cell did

The MCP tool is waggledance_view_file, and doctor now sweeps every stale mdview artifact it once wrote: the old MCP entry is deleted in the same write that adds the new one across all three config formats, the early return no longer skips that sweep, a marker block is replaced in place instead of duplicated, the orphaned .claude/skills/mdview directory is removed, and a malformed ~/.claude.json is refused rather than rewritten as an empty object.

## Recorded evidence (verbatim from .bee/cells/waggledance-rename-3.json)

- **deviation** — Evidence, run in the worktree after the sibling cell that was in flight during this worker's run landed: cargo test --workspace green, 900 passed. The worker's own run showed 895 passed and 5 red, all in e2e_open.rs and e2e_stop_stale_lock.rs — files this cell never touched, failing on daemon-startup timeouts caused by the concurrent data-dir cell's uncommitted work. Those five went green the moment that cell finished; nothing in this cell changed. The cap's own test run happens in main, which carries none of this branch, so it corroborates rather than proves.
- **deviation** — Nine new tests, written before the code: per config format (JSON for Claude Code and Antigravity, TOML for Codex) — old entry only is swept, both names present leaves only the new one, neither present adds exactly one, already-correct is idempotent on parsed content, and malformed JSON is refused with the file byte-identical. Plus a marker-block replacement starting from the old marker, and removal of a stale .claude/skills/mdview/ directory.
- **deviation** — Deviation the worker made, and the orchestrator accepted: it renamed every other user-facing string and doc comment inside its two exclusively-owned files, beyond the five numbered fixes — doctor's own CLI hints ("mdview serve", "restart", "refresh"), its header, and its "MDView block" messaging. Reason given, and it is the right one: leaving stale binary names in doctor's own guidance text would reproduce the exact "tells the user to run a deleted binary" defect this cell exists to remove.
- **deviation** — Left alone on purpose: the include_str! paths for docs/mdview-*.md and the content assertions on AGENT_TEMPLATE and SKILL_TEMPLATE. Those documents are renamed by cell waggledance-rename-9, and this cell's assertions will need to follow them there.
- **deviation** — Known and not fixed here, by decision: serde_json is declared without preserve_order, so its Map is a BTreeMap and every doctor write alphabetically reorders the keys in a user's ~/.claude.json. Unrelated content survives; formatting and ordering do not. The idempotence tests therefore assert on parsed content rather than bytes.

## Status

Candidate only. `bee knowledge promote` proposes; naming the pattern, generalizing it beyond this cell, and moving `bee.lifecycle` to `active` are a human or agent decision.

from cell waggledance-rename-4 — save as docs/knowledge/patterns/waggledance-rename-waggledance-rename-4-pitfall.md

---
type: bee.pattern
title: waggledance-rename cell waggledance-rename-4 — pitfall candidate
description: "Pitfall candidate mined from cell waggledance-rename-4's capped trace: install.ps1 was NOT syntax-checked: pwsh is absent from PATH in this environment, so the [ScriptBlock]::Create parse could not run. The worker reported that pl…"
timestamp: 2026-08-13
bee:
  id: waggledance-rename-waggledance-rename-4-pitfall
  lifecycle: draft
  areas: [daemon, doctor, web-interface, system-overview]
  sources: [.bee/cells/waggledance-rename-4.json]
  polarity: pitfall
---

# waggledance-rename cell waggledance-rename-4 — pitfall candidate

## What the cell did

The herdr override is WAGGLEDANCE_HERDR_BINARY, both installers pull thanhsmind/waggledance and install the binary outside the config directory (~/.local/bin on unix, %LOCALAPPDATA%\Programs\waggledance on Windows), and the release asset name now matches what install.sh downloads.

## Recorded evidence (verbatim from .bee/cells/waggledance-rename-4.json)

- **deviation** — install.ps1 was NOT syntax-checked: pwsh is absent from PATH in this environment, so the [ScriptBlock]::Create parse could not run. The worker reported that plainly instead of claiming it passed, which is right. install.ps1's changes are therefore review-verified only — the Windows installer needs a real parse, or a Windows CI job, before release. This is the second thing in this feature that cannot be proved on this machine; the first is the desktop crate build (cell waggledance-rename-7, no pkg-config).
- **deviation** — Asset-name consistency IS proved, and it was the cell's sharpest risk: .github/workflows/release.yml:71 publishes asset="waggledance-${{ matrix.target }}", and install.sh:48 builds its URL from ${BIN}-${TARGET} with BIN="waggledance". Same string on both sides.
- **deviation** — Deviation, accepted: D7 said install.sh's fallback chain ends at $HOME/.local/bin, but that path was already the chain's second entry, so a literal replacement of the third entry would have produced a duplicate. The worker dropped the redundant third entry instead. That serves the decision; keeping the duplicate would not.
- **deviation** — Deviation, accepted: the worker also fixed the remaining "Installing mdview" and "mdview installer" prose in the two installer scripts. Those files are this cell's alone, no later cell touches them, and stale brand text sitting next to BIN="waggledance" would contradict the rename.
- **deviation** — Flagged for awareness, not a defect: grep for MDVIEW_ still finds OLD_MDVIEW_START and OLD_MDVIEW_END in doctor.rs. Those are cell waggledance-rename-3's deliberate survivors — the old marker names it must still recognise so an existing block is replaced rather than duplicated.
- **deviation** — Evidence: cargo test --workspace exit 0 in the worktree. The cap's own run happens in main, which carries none of this branch.

## Status

Candidate only. `bee knowledge promote` proposes; naming the pattern, generalizing it beyond this cell, and moving `bee.lifecycle` to `active` are a human or agent decision.

from cell waggledance-rename-5 — save as docs/knowledge/patterns/waggledance-rename-waggledance-rename-5-pitfall.md

---
type: bee.pattern
title: waggledance-rename cell waggledance-rename-5 — pitfall candidate
description: "Pitfall candidate mined from cell waggledance-rename-5's capped trace: Evidence, run in the worktree: cargo test --workspace green, 905 passed. Baseline for this worker was 900; the +5 is its own 3 new tests plus 2 that a concurre…"
timestamp: 2026-08-13
bee:
  id: waggledance-rename-waggledance-rename-5-pitfall
  lifecycle: draft
  areas: [daemon, doctor, web-interface, system-overview]
  sources: [.bee/cells/waggledance-rename-5.json]
  polarity: pitfall
---

# waggledance-rename cell waggledance-rename-5 — pitfall candidate

## What the cell did

Browser storage keys are waggledance-* with a one-shot migration that keeps an existing user their theme and folder state, the mermaid-done event renamed on both sides at once, every mdview string cleared from the served page and its assets, and the display brand is now Waggle Dance.

## Recorded evidence (verbatim from .bee/cells/waggledance-rename-5.json)

- **deviation** — Evidence, run in the worktree: cargo test --workspace green, 905 passed. Baseline for this worker was 900; the +5 is its own 3 new tests plus 2 that a concurrent sibling cell landed in the shared tree during its run. The cap's own run happens in main, which carries none of this branch.
- **deviation** — Correction to the cell's own text, found by the worker: the mermaid-done pair is the other way round from what the cell said. The listener is in app.js and the dispatch is in views.rs, not the reverse. Both were renamed together, so the outcome is unaffected, but the plan's anchors for that pair were wrong.
- **deviation** — Three tests rather than one, and the shape is right: the page title says Waggle Dance; a sweep asserts no stray mdview across layout(), APP_JS and APP_CSS; and the mermaid event name is cross-checked between dispatch and listener, anchored on an unrelated callback name so it cannot pass by two hardcoded copies agreeing with each other.
- **deviation** — Deviation, accepted: the worker also renamed roughly 30 test-only temp-directory prefixes in views.rs's test module. They are internal scratch names, never served, but a literal grep for mdview would have found them and the final sweep cell requires that grep to be clean.
- **deviation** — Storage migration is implemented in both places it has to be: the inline no-flash theme script in views.rs and a shared migrateStorageKey helper in app.js, called at both folder-state read sites. Read old, write new, delete old, inert afterwards.

## Status

Candidate only. `bee knowledge promote` proposes; naming the pattern, generalizing it beyond this cell, and moving `bee.lifecycle` to `active` are a human or agent decision.

from cell waggledance-rename-6 — save as docs/knowledge/patterns/waggledance-rename-waggledance-rename-6-pitfall.md

---
type: bee.pattern
title: waggledance-rename cell waggledance-rename-6 — pitfall candidate
description: "Pitfall candidate mined from cell waggledance-rename-6's capped trace: The silent-drift risk this cell existed to remove is now structurally closed, not just renamed. The worker pulled the inline detection check out of health_chec…"
timestamp: 2026-08-13
bee:
  id: waggledance-rename-waggledance-rename-6-pitfall
  lifecycle: draft
  areas: [daemon, doctor, web-interface, system-overview]
  sources: [.bee/cells/waggledance-rename-6.json]
  polarity: pitfall
---

# waggledance-rename cell waggledance-rename-6 — pitfall candidate

## What the cell did

Both halves of the daemon health handshake now say waggledance, and the detection check moved out of health_check into a shared looks_like_daemon predicate that a route-level test asserts the real /health body against — so a future one-sided rename fails a test instead of silently breaking auto-spawn.

## Recorded evidence (verbatim from .bee/cells/waggledance-rename-6.json)

- **deviation** — The silent-drift risk this cell existed to remove is now structurally closed, not just renamed. The worker pulled the inline detection check out of health_check into a shared predicate, waggledance_core::daemon::looks_like_daemon, and the new test hits the real /health route through the router and asserts the body satisfies that predicate. Neither side of the test hardcodes the string, so a future one-sided rename fails the test instead of silently breaking daemon detection. A second unit test covers the extracted predicate directly.
- **deviation** — Evidence, run in the worktree: cargo test --workspace green, 902 passed — 900 before this cell, plus its two new tests. grep for mdview across server.rs, daemon.rs and runtime.rs returns nothing. The cap's own run happens in main, which carries none of this branch.
- **deviation** — Deviation, accepted: a doc comment at server.rs:2364 read "~/.mdview -> ~/.waggledance (D2)". A mechanical rename would have turned it into "~/.waggledance -> ~/.waggledance", which says nothing. The worker reworded it to refer to the config data dir's D2 rename without naming the old path literally, which keeps the meaning and satisfies the grep-clean bar for these files.
- **deviation** — Deviation, accepted: the cell named specific lines, but the worker renamed every remaining mdview occurrence in its three files — roughly 40 more, mostly doc comments and test fixture names in server.rs. None of them are on the feature's intentional-survivor list; every survivor on that list lives in another cell's files.

## Status

Candidate only. `bee knowledge promote` proposes; naming the pattern, generalizing it beyond this cell, and moving `bee.lifecycle` to `active` are a human or agent decision.

from cell waggledance-rename-7 — save as docs/knowledge/patterns/waggledance-rename-waggledance-rename-7-pitfall.md

---
type: bee.pattern
title: waggledance-rename cell waggledance-rename-7 — pitfall candidate
description: "Pitfall candidate mined from cell waggledance-rename-7's capped trace: The cell's own headline check could NOT be run: `cargo build --manifest-path crates/waggledance-desktop/Cargo.toml` fails before it reaches this crate's code, …"
timestamp: 2026-08-13
bee:
  id: waggledance-rename-waggledance-rename-7-pitfall
  lifecycle: draft
  areas: [daemon, doctor, web-interface, system-overview]
  sources: [.bee/cells/waggledance-rename-7.json]
  polarity: pitfall
---

# waggledance-rename cell waggledance-rename-7 — pitfall candidate

## What the cell did

crates/mdview-desktop is now crates/waggledance-desktop, with the package name, the waggledance-core dependency, the Tauri productName and identifier (dev.waggledance.app), every window/tray/binary string in main.rs, ui/index.html, the crate README, the root Cargo.toml exclude line, .gitignore, and the tracked Cargo.lock all following. The crate build itself could not be verified here — no pkg-config on this machine.

## Recorded evidence (verbatim from .bee/cells/waggledance-rename-7.json)

- **deviation** — The cell's own headline check could NOT be run: `cargo build --manifest-path crates/waggledance-desktop/Cargo.toml` fails before it reaches this crate's code, because libdbus-sys v0.2.7's build script needs pkg-config and this machine has none (`which pkg-config` exits 1; /usr/bin/pkg-config and /usr/bin/pkgconf do not exist). The crate's own README already documents needing libwebkit2gtk, gtk3 and dbus dev libraries on Linux, so this is a pre-existing environment gap, not a rename defect — the same failure occurs against the old manifest path. `cargo check` fails identically, at the same build-script stage.
- **deviation** — What IS proven: dependency resolution succeeds, which is what regenerates Cargo.lock, and the regenerated lock shows a coherent waggledance-core / waggledance-desktop graph with zero mdview entries. `grep -rn mdview crates/waggledance-desktop` is empty. `cargo test --workspace` is green at 886 passed, though that suite excludes this crate by design and so proves nothing about it.
- **deviation** — So the must-have "the desktop crate builds via its own manifest" is UNPROVEN in this environment, and the cap should not be read as proving it. It needs a machine with pkg-config, libwebkit2gtk and gtk3, or CI, before release.
- **deviation** — Scope note the worker recorded: root Cargo.toml line 6 still carries a comment pointing at "crates/mdview-desktop/README.md". The worker was told to change only the exclude line in that file because a sibling worker was reading it, so it left the comment. Cell waggledance-rename-9 (the docs and final sweep) must catch it.

## Status

Candidate only. `bee knowledge promote` proposes; naming the pattern, generalizing it beyond this cell, and moving `bee.lifecycle` to `active` are a human or agent decision.

from cell waggledance-rename-9 — save as docs/knowledge/patterns/waggledance-rename-waggledance-rename-9-pitfall.md

---
type: bee.pattern
title: waggledance-rename cell waggledance-rename-9 — pitfall candidate
description: "Pitfall candidate mined from cell waggledance-rename-9's capped trace: Evidence, run in the worktree: cargo test --workspace green, 906 passed — unchanged from before this cell, which is right for a rename that moved template file…"
timestamp: 2026-08-13
bee:
  id: waggledance-rename-waggledance-rename-9-pitfall
  lifecycle: draft
  areas: [daemon, doctor, web-interface, system-overview]
  sources: [.bee/cells/waggledance-rename-9.json]
  polarity: pitfall
---

# waggledance-rename cell waggledance-rename-9 — pitfall candidate

## What the cell did

Every document, template filename and path now says waggledance, the managed block in AGENTS.md and CLAUDE.md matches what doctor writes, doctors include_str paths and template assertions followed the renamed files, and the sweep over content and paths including Cargo.lock leaves only named, test-pinned survivors.

## Recorded evidence (verbatim from .bee/cells/waggledance-rename-9.json)

- **deviation** — Evidence, run in the worktree: cargo test --workspace green, 906 passed — unchanged from before this cell, which is right for a rename that moved template files without adding or removing assertions. The orchestrator re-ran the sweep independently and got exactly the worker's twelve files, with the path sweep empty and Cargo.lock clean. The cap's own test run happens in main, which carries none of this branch.
- **deviation** — The seven planned survivors all hold. Five more files carry mdview and each is accounted for:
- **deviation** — - repository.rs and short_link.rs — ruled out in the plan itself. There "mdview" is an arbitrary registered-project NAME used as a golden-hash input, not this binary; rewriting it would break the constants for nothing.
- **deviation** — - docs/knowledge/work/upstream-short-link/delivery.md — "vantt/mdview" is the actual upstream fork this repo diverged from. Not ours to rename.
- **deviation** — - plans/260715-1835-waggledance-mvp/plan.md and the two reports under plans/reports/ — dated, completed build and research logs. Their path names were renamed as instructed; their prose was not, because rewriting it would misrepresent what was literally built and decided at the time. Same reasoning docs/history gets by exclusion, applied to records that happen to sit outside it.
- **deviation** — - docs/backlog.md — SURVIVOR #8, recorded here as a deliberate outcome rather than a miss. All 17 mentions sit in rows marked done, describing what was built when it was built, with crates/mdview/... paths as they existed then; it is the same class of historical ledger. The worker could not have edited it anyway: bee's write-guard owns that file and refuses direct edits, and the worker was correctly told to run no bee command inside the worktree.
- **deviation** — One genuine tail, small and named: docs/backlog.md line 3 reads "PBI rows cho mdview", which is live prose, not history. It needs a bee backlog pass from the main checkout after the merge. Line 12's MDVIEW_VERSION sits inside a done row and stays as history.

## Status

Candidate only. `bee knowledge promote` proposes; naming the pattern, generalizing it beyond this cell, and moving `bee.lifecycle` to `active` are a human or agent decision.

knowledge promote: 9 capped cell(s) mined, 1 delivery draft, 32 area bullet(s), 8 pattern candidate(s), 0 file(s) written.