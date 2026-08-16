---
type: bee.delivery
title: review-p1-fixes — delivery
description: "Delivery record for work item review-p1-fixes: 6 capped cell(s), 0 recorded deviation(s) — the P1 security findings closed."
timestamp: 2026-08-16
bee:
  id: review-p1-fixes-delivery
  lifecycle: active
  areas: [agent-terminal, web-interface, bee-cockpit, system-overview]
  required_context: [docs/specs/agent-terminal.md]
  sources: [.bee/lanes/review-p1-fixes.json, .bee/cells/review-p1-fixes-1.json, .bee/cells/review-p1-fixes-2.json, .bee/cells/review-p1-fixes-3.json, .bee/cells/review-p1-fixes-4.json, .bee/cells/review-p1-fixes-5.json, .bee/cells/review-p1-fixes-6.json]
---

# review-p1-fixes — Delivery

## What shipped

- **review-p1-fixes-1** — Added a require_loopback_host middleware layer returning 421 for non-loopback/missing Host; 13 tests cover loopback pass-through, foreign-Host 421 with no side effect on config/terminal-config/register/unregister, and missing-Host 421. Closes DNS-rebinding + CSRF findings. cargo test --workspace green (1021). (1 file(s) changed)
- **review-p1-fixes-2** — Escaped the title inside layout() with esc(), covering all callers at one sink; added tests for injection, ampersand/quote escaping, plain-title no-op, and the reflected search_page path. (1 file(s) changed)
- **review-p1-fixes-3** — Closed the open data-* sanitizer allowlist (only data-sourcepos survives) and gated app.js's data-term-base reads behind a same-origin /p/<project>/... check (2 file(s) changed)
- **review-p1-fixes-4** — Gate a feature URL segment through validate_feature_name before joining it onto the archive path in read_archived_cells, covering bee_feature_detail's own call; new test proves traversal/separator/empty features read nothing while a normal slug still reads. (1 file(s) changed)
- **review-p1-fixes-5** — fmt clean, ~24 clippy errors fixed across bee.rs/ansi.rs plus files surfaced by cells 1-4/6 (main.rs, server.rs, views.rs, herdr/mod.rs, herdr/socket.rs, supervisor.rs, watcher.rs), commands.test widened to the CI triple, all three gates green (1022 tests, 5 suites) (1 file(s) changed)
- **review-p1-fixes-6** — Landed a Rust #[test] boundary assertion (no node-free JS harness exists in repo) proving shouldReload's term-screen guard: Kanban/Projects tabs never render .term-screen, Terminals tab with a selected agent pane always does. cargo test --workspace green (1022 passed). (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **review-p1-fixes-1** — server tests: loopback Hosts (127.0.0.1, localhost, [::1]) pass through; Host evil.tld / attacker IP to the config, terminal-config, register/unregister POSTs and a plain GET each return 421 with the handler's side effect proven absent; a missing Host header returns 421.
- **review-p1-fixes-2** — view tests: a title carrying `</title><script>` renders escaped; `&` and `"` escaped; plain titles unchanged apart from escaping; the reflected search_page path asserts an escaped `<title>`.
- **review-p1-fixes-3** — sanitizer test: hostile `data-term-base`/non-allowlisted data-* attributes are stripped while renderer-emitted data-sourcepos survives; the app.js same-origin check is a JS-only guard recorded with its manual browser check (a hostile data-term-base is not fetched).
- **review-p1-fixes-4** — bee tests: feature values `../../etc`, `..%2F..`, `a/b`, and `` each read nothing and touch no file outside the archive root; a normal slug still reads its archived cells.
- **review-p1-fixes-5** — `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` green under the widened commands.test.
- **review-p1-fixes-6** — Rust boundary assertion over shouldReload's three cases passes (Kanban/Projects tabs never render .term-screen; Terminals tab with a selected pane always does).

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work review-p1-fixes` from 6 capped cell trace(s). Accepted at the compounding pass on 2026-08-16 and saved here as the factual delivery record — it is not a specification. The proposal's five area-update bullets were checked against the living specs and found already merged by the feature's own scribing sync: the loopback-host boundary and the text-never-markup guarantee are stated in `docs/specs/agent-terminal.md` ("Business Rules", commit 0730429), the feature-name validation before any disk join in `docs/specs/bee-cockpit.md` ("It renders nothing that identifies a filesystem outside the project"), and the terminal-screen live-reload guard in `docs/specs/system-overview.md` ("Live reload"). No pattern candidates were proposed.
