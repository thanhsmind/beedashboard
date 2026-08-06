---
type: bee.delivery
title: agent-terminal — delivery
description: "Delivery record proposed by bee knowledge promote for work item agent-terminal: 26 capped cell(s), 5 recorded deviation(s)."
timestamp: 2026-08-06
bee:
  id: agent-terminal-delivery
  lifecycle: active
  areas: [agent-terminal, settings, web-interface, system-overview, bee-cockpit, reading-map]
  required_context: [docs/history/agent-terminal/CONTEXT.md, docs/history/agent-terminal/plan.md]
  sources: [docs/history/agent-terminal/CONTEXT.md, docs/history/agent-terminal/plan.md, .bee/cells/archive/agent-terminal/agent-terminal-1.json, .bee/cells/archive/agent-terminal/agent-terminal-2.json, .bee/cells/archive/agent-terminal/agent-terminal-3.json, .bee/cells/archive/agent-terminal/agent-terminal-4.json, .bee/cells/archive/agent-terminal/agent-terminal-5.json, .bee/cells/archive/agent-terminal/agent-terminal-6.json, .bee/cells/archive/agent-terminal/agent-terminal-7.json, .bee/cells/archive/agent-terminal/agent-terminal-8.json, .bee/cells/archive/agent-terminal/agent-terminal-9.json, .bee/cells/archive/agent-terminal/agent-terminal-10.json, .bee/cells/archive/agent-terminal/agent-terminal-11.json, .bee/cells/archive/agent-terminal/agent-terminal-12.json, .bee/cells/archive/agent-terminal/agent-terminal-13.json, .bee/cells/archive/agent-terminal/agent-terminal-14.json, .bee/cells/archive/agent-terminal/agent-terminal-15.json, .bee/cells/archive/agent-terminal/agent-terminal-16.json, .bee/cells/archive/agent-terminal/agent-terminal-17.json, .bee/cells/archive/agent-terminal/agent-terminal-18.json, .bee/cells/archive/agent-terminal/agent-terminal-19.json, .bee/cells/archive/agent-terminal/agent-terminal-20.json, .bee/cells/archive/agent-terminal/agent-terminal-21.json, .bee/cells/archive/agent-terminal/agent-terminal-22.json, .bee/cells/archive/agent-terminal/agent-terminal-23.json, .bee/cells/archive/agent-terminal/agent-terminal-24.json, .bee/cells/archive/agent-terminal/agent-terminal-25.json, .bee/cells/archive/agent-terminal/agent-terminal-26.json]
---

# agent-terminal — Delivery

## What shipped

- **agent-terminal-1** — Config resolves its data directory through an injectable override so settings handlers can be tested without touching the real ~/.mdview (2 file(s) changed)
- **agent-terminal-2** — Ported herdr client into crates/mdview (mdview-core stays async-runtime-free) and the path boundary into crates/mdview-core/src/paths_boundary.rs, both with their existing tests green (12 file(s) changed)
- **agent-terminal-3** — Add terminal_auth module: token store (0600 file), constant-time verify, session set, reveal-once masking, rotation clears sessions; opaque-404 extractor generic over HasTerminalAuth; no route mounted (2 file(s) changed)
- **agent-terminal-4** — Issued and rotated the terminal token on the settings page (reveal-once, mask-after), and gated the D7 switches behind a terminal_auth-only endpoint separate from POST /api/config (3 file(s) changed)
- **agent-terminal-5** — Added the gated GET /p/:id/_terminal pane list (D2/D4/D6), the project tab strip, and the MethodGate carry-over fix for POST /api/terminal-config (2 file(s) changed)
- **agent-terminal-6** — Add the gated screen-poll endpoint (herdr-go's ScreenBody shape) plus the app.js poll loop, and enforce TerminalConfig.enabled across the whole terminal route family while keeping settings and the switch endpoint reachable (3 file(s) changed)
- **agent-terminal-7** — Close the terminal-auth rotation race with a single-lock verify_and_mint, add a MethodGate extractor closing the method-mismatch 404/405 oracle, make the token write atomic (temp+rename, 0600), and surface Windows' unprotected state as a queryable flag (2 file(s) changed)
- **agent-terminal-8** — Login route is the only caller of verify_and_mint and only place a session cookie is minted; rotation no longer mints, gated on current session once configured; Terminal tab present on every project (3 file(s) changed)
- **agent-terminal-9** — Add gated POST input/keys endpoints for pane replies (D3), the reply bar + key buttons on the terminal page, and app.js wiring; submit-flag defaults to staged-not-sent (3 file(s) changed)
- **agent-terminal-10** — Added the gated Unassigned group (routes, page, screen/input/keys, project-list presence marker) with route-level tests covering the partition, gating, and unauthenticated home page; cargo test --workspace green (337 passed). (2 file(s) changed)
- **agent-terminal-11** — Pinned the unassigned route family's guards (session/method/switch), the socket-layer send-then-submit split, the token-mount method gate, MethodGate isolation, and fail-closed unassigned listing with a key-list bound (2 file(s) changed)
- **agent-terminal-12** — Server-side ANSI-to-HTML translation renders coloured/styled terminal screens; unrecognised escapes dropped, text escaped before markup (6 file(s) changed)
- **agent-terminal-13** — Added gated pane/agent creation routes with server-side workspace resolution and preset-based agent start; terminal page offers configured preset labels (3 file(s) changed)
- **agent-terminal-14** — ANSI parser now drops DCS/APC/PM/SOS bodies whole and re-emits malformed-CSI abort bytes as text; creation routes gain MethodGate isolation and create_error_response coverage tests (2 file(s) changed)
- **agent-terminal-15** — Ported herdr-go's gap-free transcript tailer into mdview-core with its full test suite and cursor path-escape guard (2 file(s) changed)
- **agent-terminal-16** — Added the Transcript tab (D9) beside Terminal with a gated, cursor-based activity endpoint sharing the terminal family's full guard stack (3 file(s) changed)
- **agent-terminal-17** — Ported the status watcher, herdr supervisor and at-least-once notification outbox (Notifier trait + null/Telegram) from herdr-go, inert — main.rs only declares the modules, guarded by a source-scan test proving nothing constructs or starts them (9 file(s) changed)
- **agent-terminal-18** — Wired the supervisor and Telegram notifier behind their D7 switches via a new live-reconciling TerminalBackground manager; the credential follows the terminal-token secret-file pattern (4 file(s) changed)
- **agent-terminal-19** — Fixed all 7 transcript reader defects: structural cursor path guard, oversized-record advance, truncation divider, boundary-correct backfill, clipped unknown types, honest poll-cap wording — 21 new/updated tests, full workspace suite green (1 file(s) changed)
- **agent-terminal-20** — Pinned the transcript page's session/method gates with isolating tests and fixed the transcript poller's double-append and silent-failure defects (2 file(s) changed)
- **agent-terminal-21** — Fixed the toothless off-switch (real tick-based proof + cancellation-before-spawn + wait-for-previous-task), added exponential backoff with per-step logging, randomised the credential temp name, logged its write failures, added an owner-only-mode test, and corrected four stale module docs (7 file(s) changed)
- **agent-terminal-22** — Fixed the transcript poller's in-flight guard to clear only on settle (both success and failure paths), replaced two toothless grep-based tests with block-pinned assertions, and made the notify outbox open lazily only when the notify switch is on; the off-path cancellation assertions (item 5) remain bookkeeping-only pending agent-terminal-21's not-yet-landed real side effect in main.rs, named as a deviation rather than fabricated. (2 file(s) changed)
- **agent-terminal-23** — Aligned the opening backfill's oversized-record handling with the tailer's so a still-writing record no longer leaves the cursor mid-record and storms a truncation divider; separated the cursor guard's empty-name and extension checks (stem-based) so each is independently load-bearing, with a new .jsonl:0 fixture proving it. (1 file(s) changed)
- **agent-terminal-24** — Surface a failed Telegram credential save as failure not saved, and prove the off-assertions observe the tick side effect rather than the bookkeeping slot (2 file(s) changed)
- **agent-terminal-25** — Corrected every no-auth claim in scope to name D4's carve-out, added docs/specs/agent-terminal.md, listed it in reading-map.md, and updated README (9 file(s) changed)
- **agent-terminal-26** — Corrected settings-gating, creation-body, and pane-listing claims; added 8 missing behaviours; stripped implementation vocabulary; distinguished agent from pane with a named open gap (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **agent-terminal-1** — `cargo test --workspace`
- **agent-terminal-2** — `cargo test --workspace`
- **agent-terminal-3** — `cargo test --workspace`
- **agent-terminal-4** — `cargo test --workspace`
- **agent-terminal-5** — `cargo test --workspace`
- **agent-terminal-6** — `cargo test --workspace`
- **agent-terminal-7** — `cargo test --workspace`
- **agent-terminal-8** — `cargo test --workspace`
- **agent-terminal-9** — `cargo test --workspace`
- **agent-terminal-10** — `cargo test --workspace`
- **agent-terminal-11** — `cargo test --workspace`
- **agent-terminal-12** — `cargo test --workspace`
- **agent-terminal-13** — `cargo test --workspace`
- **agent-terminal-14** — `cargo test --workspace`
- **agent-terminal-15** — `cargo test --workspace`
- **agent-terminal-16** — `cargo test --workspace`
- **agent-terminal-17** — `cargo test --workspace`
- **agent-terminal-18** — `cargo test --workspace`
- **agent-terminal-19** — `cargo test --workspace`
- **agent-terminal-20** — `cargo test --workspace`
- **agent-terminal-21** — `cargo test --workspace`
- **agent-terminal-22** — `cargo test --workspace`
- **agent-terminal-23** — `cargo test --workspace`
- **agent-terminal-24** — `cargo test --workspace`
- **agent-terminal-25** — `cargo test --workspace`
- **agent-terminal-26** — `cargo test --workspace`

## Deviations

- **agent-terminal-7** — Removed the public verify(&str) -> bool entirely rather than merely privatizing it (no caller outside this module used it); kept mint_session() public because server.rs's rotate_terminal_token (owned concurrently by cell agent-terminal-4) calls it directly after rotate() on the settings-page rotation flow, which is not the verify()+mint_session() login composition the rotation race targeted -- old callers keep compiling unchanged.
- **agent-terminal-7** — Closed the rotation race with a single Mutex shared by rotate() and the new verify_and_mint() (mutual exclusion across the write+clear and read+compare+insert) rather than a token-generation counter -- both were authorised by the cell; the lock is the simpler structural fix.
- **agent-terminal-7** — Recorded, per the cell's closing instruction, that agent-terminal-3's first truth ("wrong token yields an opaque 404") is now a login-route obligation satisfied through verify_and_mint's caller contract (None on any failure) -- documented in terminal_auth.rs's module doc under "Note for the cell that mounts the first terminal login route", so the next cell that adds a login route inherits it explicitly rather than re-deriving it.
- **agent-terminal-7** — The method-mismatch fix is MethodGate<M> (an extractor) plus Get/Post marker types, not a MethodRouter fallback handler -- verified against axum 0.7.9 source (routing/method_routing.rs) that MethodRouter::fallback still lets axum attach an Allow header derived from registered methods even through a custom fallback, so no fallback-handler choice can close the oracle; routes must be mounted with axum::routing::any(handler) and MethodGate<Get>/MethodGate<Post> as the handler's first extractor. Documented in MethodGate's doc comment for the cells that mount terminal routes.
- **agent-terminal-7** — Windows-silence fix is a const fn token_file_permissions_enforced() -> bool (a compile-time platform fact via cfg!(unix)) rather than a stored runtime flag, since the property doesn't vary at runtime on a given build.

## Provenance

Proposed by `bee knowledge promote --work agent-terminal` from 26 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/agent-terminal/CONTEXT.md`, `docs/history/agent-terminal/plan.md`. Every line above is copied from a trace or from the work item.

Accepted at the feature's close on 2026-08-06 and saved here unedited: this is the delivery record, a factual account of what shipped and how each cell was verified. It is not a specification — the living behaviour specs are `docs/specs/agent-terminal.md` and the area specs listed in `docs/specs/reading-map.md`. The same close reviewed the proposal's other two sections: its area-update bullets were checked against the living specs and the real gaps applied there, and its pattern candidates were rejected as auto-mined drafts carrying no generalized lesson, with the one durable trap moved to `docs/history/learnings/critical-patterns.md` instead.
