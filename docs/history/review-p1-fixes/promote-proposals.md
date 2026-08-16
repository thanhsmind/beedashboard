promote proposal for work item "review-p1-fixes" (.bee/logs/scribing-runs.jsonl + .bee/lanes/review-p1-fixes.json) — 6 capped cell(s): review-p1-fixes-1, review-p1-fixes-2, review-p1-fixes-3, review-p1-fixes-4, review-p1-fixes-5, review-p1-fixes-6
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/review-p1-fixes.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/review-p1-fixes/delivery.md

---
type: bee.delivery
title: review-p1-fixes — delivery
description: "Delivery record proposed by bee knowledge promote for work item review-p1-fixes: 6 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: review-p1-fixes-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/review-p1-fixes.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/review-p1-fixes.json, .bee/cells/review-p1-fixes-1.json, .bee/cells/review-p1-fixes-2.json, .bee/cells/review-p1-fixes-3.json, .bee/cells/review-p1-fixes-4.json, .bee/cells/review-p1-fixes-5.json, .bee/cells/review-p1-fixes-6.json]
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

- **review-p1-fixes-1** — `cargo test --workspace green. New server tests: a request with Host 127.0.0.1:7700 (and localhost, [::1]) passes through to its normal response on a sample GET and on POST /api/config; a request with Host evil.tld (or an attacker IP) to POST /api/config, POST /api/terminal-config, POST a register/unregister route, and a plain GET each returns 421 and the handler's side effect did NOT happen (config unchanged / project still registered); a request with no Host header returns 421. Reuse the existing test harness that builds the router with a fixture state.`
- **review-p1-fixes-2** — `cargo test --workspace green. New view test: layout() given a title containing </title><script>alert(1)</script> renders it with the angle brackets escaped (no live </title> or <script> in output); a title with & and " is escaped; a plain title renders unchanged apart from escaping. If a reflected path is cheap to assert, add: search_page with q=</title><script> produces an escaped <title>. Existing layout/page tests stay green.`
- **review-p1-fixes-3** — `cargo test --workspace green. New render/sanitizer test: a markdown input containing <pre class="term-screen" data-pane-id="x" data-term-base="https://evil.tld/x"> passes through sanitize() with data-term-base (and any non-allowlisted data-*) STRIPPED, while a legitimately-emitted data-* the renderer produces (e.g. data-sourcepos) survives. Assert on the sanitized HTML string. The app.js same-origin check has no Rust harness -- record it as a JS-only guard the way home-terminal-header-2 did, naming the manual browser check (a hostile data-term-base is not fetched).`
- **review-p1-fixes-4** — `cargo test --workspace green. New bee.rs test: read_archived_cells with feature values '../../etc', '..%2F..' (already-decoded to ../..), 'a/b', and '' each return an empty Vec and touch no file outside root/.bee/cells/archive; a normal feature slug still reads its archived cells from a fixture. If a sibling reader shares the segment, it gets the same test.`
- **review-p1-fixes-5** — `Fresh and quoted: cargo fmt --all --check exits clean; cargo clippy --workspace --all-targets -- -D warnings exits clean (0 errors); cargo test --workspace green. This cell's own cap must run the widened commands.test and pass all three. Quote the tail of each in the cap message.`
- **review-p1-fixes-6** — `cargo test --workspace green. Either: a new automated assertion over shouldReload's three cases passes; OR an explicit recorded-gap note in the cell trace naming the manual verification plus a bee backlog add for the JS harness. State which path was taken and why in the cap message.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work review-p1-fixes` from 6 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/review-p1-fixes.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "review-p1-fixes" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-16T01:12:52.802Z), the work item declares no bee.areas.

area agent-terminal:
  - [review-p1-fixes-1] Added a require_loopback_host middleware layer returning 421 for non-loopback/missing Host; 13 tests cover loopback pass-through, foreign-Host 421 with no side effect on config/terminal-config/register/unregister, and missing-Host 421. Closes DNS-rebinding + CSRF findings. cargo test --workspace green (1021). — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/review-p1-fixes-1.json)
  - [review-p1-fixes-2] Escaped the title inside layout() with esc(), covering all callers at one sink; added tests for injection, ampersand/quote escaping, plain-title no-op, and the reflected search_page path. — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/review-p1-fixes-2.json)
  - [review-p1-fixes-3] Closed the open data-* sanitizer allowlist (only data-sourcepos survives) and gated app.js's data-term-base reads behind a same-origin /p/<project>/... check — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/review-p1-fixes-3.json)
  - [review-p1-fixes-4] Gate a feature URL segment through validate_feature_name before joining it onto the archive path in read_archived_cells, covering bee_feature_detail's own call; new test proves traversal/separator/empty features read nothing while a normal slug still reads. — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/review-p1-fixes-4.json)
  - [review-p1-fixes-6] Landed a Rust #[test] boundary assertion (no node-free JS harness exists in repo) proving shouldReload's term-screen guard: Kanban/Projects tabs never render .term-screen, Terminals tab with a selected agent pane always does. cargo test --workspace green (1022 passed). — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/review-p1-fixes-6.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 6 capped cell(s) mined, 1 delivery draft, 5 area bullet(s), 0 pattern candidate(s), 0 file(s) written.