---
artifact_contract: bee-plan/v1
mode: high-risk
# approved_gate2: <unset>
---

# Plan: Bee Board — PM View

Mode: `high-risk` — 4 risk flags: public-contracts, covered-contract-change, proof-weakening, multi-domain
Why this is the least workflow that protects the work: the redesign replaces the entire rendered surface of a page whose correctness is currently held by ~47 route tests, most of which assert the markup being deleted — so the danger is not writing the new board, it is losing the invariants the old tests were carrying while nobody is looking.

## Requirements (from CONTEXT.md)

- **D1** — English labels; the source spec's structure and semantics, not its Vietnamese wording.
- **D2** — Delivery speed, worktrees and workspaces survive, folded into the new layout.
- **D3** — Cards link to the existing `/_bee/cell/:id` and `/_bee/feature/:feature` pages; no drawer.
- **D4** — File-read-only: everything comes from reading `.bee/**` plus `docs/history/<feature>/promote-proposals.md`; the bee CLI is never executed.
- **D5** — Fixed question order: lifecycle stepper → headline numbers → what is being worked on now beside what needs attention → all work by phase → supporting panels. Empty sections render an honest empty state, never zeros-as-measurements, never disappearance.
- **D6** — "Needs attention" is generated and severity-ordered, each rule independent, each item naming a suggested action; empty says so in one line.
- **D7** — Independent review is presented as user-invoked, never as pending automatic work.
- **D8** — `dropped` cells count toward no denominator and no total.
- **D9** — Nothing identifying a filesystem outside the project is rendered.

## Discovery

Three parallel inspections of the real stores and the existing code.

**The store carries far more than the reader currently parses, and all of it is on disk.** `.bee/state.json` already holds `approved_gates`, `route` (class/lane/flags/product_files/rationale/updated_at), `next_action`, `last_scribing_run` and `last_compounding_run` — none of which `read_snapshot` looks at today. Verified against both `/home/thanhsmind/projects/goglbe/beehive/.bee/state.json` (bee 2.2.2) and this repo's (bee 2.1.15).

**A lane record is a full parallel copy of its feature's state, not a stub.** Each `.bee/lanes/<feature>.json` carries its own `phase`, `approved_gates`, `mode`, `next_action` and `created_at`; most also carry `last_scribing_run` (35 of beehive's 46) and some `last_compounding_run` (16 of 46) — their absence is itself the signal that a feature was never captured. This answers CONTEXT.md's second deferred question with a yes. It also exposes two traps. **This repo has no `.bee/lanes/` directory at all**, while beehive has 46 lane files — so the phase board must be correct with one feature to place and with forty-seven. And beehive's globally active feature has no lane file of its own, so the board places the union `lanes ∪ {state.feature}`, never the lane list alone; a board that trusted `.bee/lanes/` would omit the one feature actually being worked on.

**`gate_bypass` is not in `state.json`, and it is not in one file either.** It appears in `.bee/config.json` — `false` here, `"total"` in beehive, so the value is read defensively and normalized. But `.bee/config.local.json` exists on disk here too, and bee's own CLI describes it as a machine-local overlay whose values are what keys "resolve to by default". The effective bypass level is therefore an overlay resolution, not a field. **The board renders only what `config.json` literally records and labels it as such** — a claim about the tracked setting, not about the effective one. Rendering an effective level would mean re-implementing bee's resolution order and being silently wrong on any machine that overlays it, inside the one panel whose whole job is to be trustworthy. Reading it still needs a new reader and a new fixture builder.

**`state.json` carries one more key the stepper needs.** This repo's own file holds `gate_revoked_at` (`{"execution": "2026-08-05T09:51:47.038Z"}`) — a gate that was approved and then revoked. The lifecycle stepper reads it, so a revoked gate does not render as approved.

**Review state is derivable from files, but not from the file the source spec names.** `.bee/review-candidates.jsonl` carries no status field of any kind — every row is `type:"candidate"` with `{id, date, feature, head, mode, baseline, cells[]}`. Reviewed-ness lives in `.bee/reviews/*.json`, where each session carries `included[]` (the cells and commits under review), `findings[]` with a `severity` of `P1`/`P2`/`P3`, and a `decision` whose `status` is `approved`, `blocked` or `pending`. Joining the two gives more than the source spec asked for: not just how many candidates are unreviewed, but how many P1 findings are open right now. Evidence: ten review sessions read in beehive; `unreviewed-batch-20260805-a.json` has `decision.status: "pending"` with 7 findings, `codex-agent-wait-loop-review-20260715-r2.json` has `decision.status: "blocked"` with `p1_count: 1`.

**The reader has one parsing convention and no serde derives for store files.** Every `.bee/` file is walked by hand through `serde_json::Value` (`bee.rs:621` `read_state`, `bee.rs:901` `read_lanes` as the directory-listing template, `bee.rs:930` `parse_lane` as the per-file template). A missing file is silent and normal; a read or parse error pushes one line onto `read_errors` and continues — nothing ever returns `Err` or panics (`bee.rs:475-480`).

**D9 is held today only for fields that are entirely a path, and every field this feature adds is free text.** `relativize` (`bee.rs:1356-1368`) opens with `let p = Path::new(s); if !p.is_absolute() { return s.to_string(); }` — so a string that merely *contains* an absolute path mid-sentence is returned untouched, because the sentence as a whole is not an absolute path. Everything the new sections render is exactly that shape: `state.next_action`, `route.rationale`, `HANDOFF.json`'s `next_action`, and review findings prose. This repo's own handoff is a five-sentence paragraph naming filesystem paths. The existing leak test (`no_absolute_path_or_fixture_root_in_response_body`, `server.rs:2547`) plants absolute paths only in a cell's `files[]` and `trace.worker` — the two already-relativized path fields — so it would pass while a handoff leaked the operator's home directory onto the page. **D9 is therefore new work here, not an inherited guarantee**, and the plan treats it as such below.

**Time is taken once and threaded down.** `read_snapshot` calls `OffsetDateTime::now_utc()` at `bee.rs:547` and passes that single `now` into `read_sessions`, `read_worktrees` and `parse_session`. New derivations that need "how long ago" take `now` as a parameter rather than reading the clock themselves.

## Approach

**Recommended: extend the reader with typed, independently-testable derivations, and rebuild the view as a composition of small section functions over that snapshot.** The snapshot stays the single seam. Every new number — attention items included — is computed in `mdview-core` as a pure function over already-read data, in the shape `compute_running_workers` (`bee.rs:585`) already establishes, so it can be unit-tested without an HTTP request and without markup. `views.rs` then only arranges and escapes. This keeps D6's attention rules provable in isolation, which matters because they are the part most likely to be quietly wrong.

*Rejected — compute the attention rules in `views.rs` alongside the markup.* Cheaper to write, but every rule would then only be testable through a rendered page, which is exactly the coupling that makes the current test suite fragile.

*Rejected — a client-side render from an injected JSON blob (the source spec's architecture).* It buys the live refresh we explicitly deferred, costs the no-JS server-rendered page mdview otherwise is, and reopens the `</script>` escaping hazard for free-text cell titles.

*Rejected — a parallel `_bee2` route during the rewrite.* Two boards means two truths and a migration; the tests are the safety net here, not a spare page.

**Free text is scrubbed, not relativized (D9).** Every free-text field the new sections render passes through a scrubber that finds absolute paths *inside* a string and reduces them, rather than through `relativize`, which only handles a string that is wholly a path. The scrubber lives beside `relativize` in `mdview-core`, is unit-tested directly, and is applied at the reader, not the view — so the snapshot itself never carries an absolute path and no future render site can forget to call it.

**Risk map**

| Component | Risk | Proof needed |
|---|---|---|
| Free-text fields leaking absolute paths (D9) | **HIGH** | One embedded-absolute-path case per new free-text field — `next_action`, `route.rationale`, handoff `next_action`, review finding prose — asserted on the rendered body, plus direct unit tests of the scrubber. The existing leak test does not cover this and must not be mistaken for coverage. |
| Deleting board-markup tests | **HIGH** | Every deleted test is either re-expressed against the new markup or explicitly named as testing something that no longer exists. The invariant tests (no absolute path, byte-identical `.bee` tree, 404 rules) are never touched. |
| Review join (candidates × sessions) | **MEDIUM** | Unit tests over fixture stores: a candidate in no session, in a pending session, in an approved session, and a session naming a cell that no longer exists. |
| Scribing-debt derivation | **MEDIUM** | A cell carries no "was this captured" flag — debt is inferred from capped `behavior_change` cells whose feature has no `last_scribing_run` naming it. Pinned against both stores. |
| Phase board with zero lane files | **MEDIUM** | This repo is the fixture: one active feature, no `.bee/lanes/`, must render honestly rather than empty. |
| Store shape drift across bee versions | **LOW** | Hand-rolled `Value` walking already tolerates missing keys; a fixture per known drift (`route.feature` vs `route.demoted_at`). |
| `gate_bypass` value type | **LOW** | Observed as both `false` (this repo) and `"total"` (beehive). Normalized on read, both pinned. |

**One honest report the board must make.** `.bee/HANDOFF.json` has no consumed-marker: this repo's handoff was written at 08:08 today naming work that has since been finished, and `bee status` still reports it. The board reports what the store says and dates it, so a stale pause reads as a stale pause rather than as current news. It does not invent a "probably resolved" judgement.

## Shape

**Feature outcome.** A project manager opens `/p/<project>/_bee` and, without knowing a single bee term, reads down the page and learns what was built, what is moving, what is next, and what is stuck — with every claim traceable to a link.

**Repo-reality basis.** `views::bee_board_page` itself is `views.rs:639-723` — a single `format!` over a static template with named placeholders (`{running} {worktrees} {velocity} {doing}{waiting}{stuck} {done} {panels} {errors}`), which is why new sections can be inserted without disturbing the helpers that fill the old ones. The helpers it calls occupy `views.rs:739-1452`. One of them, `bee_status_tone` (`views.rs:1446`), is **also** called by `bee_cell_page` at `views.rs:1576`: it is shared with a detail page D3 freezes, so it is changed only additively, never rewritten. `read_snapshot` is `bee.rs:481-575`. Board behaviour is tested through the HTTP route in `mod bee_route_tests` (`server.rs:2294`) and the reader directly in `bee.rs`'s own test module (`bee.rs:1385`).

| Epic | Capability / risk area | Why it exists | Slices | Proof needed |
|---|---|---|---|---|
| **E1 — The page answers in order** | D5's reading order, end to end on real data | Without this the rest is decoration; it is also the smallest thing that is genuinely the new board | S1a, S1b | The new page renders on this repo's own store and on beehive's, and states the active feature's gates, progress and next action correctly in both |
| **E2 — Every feature placed by phase** | D5's by-phase board, replacing the four buckets | The four buckets answer "what cell state" — a manager asks "what feature, how far along" | S2 | Beehive's forty-seven place correctly, including the active feature that has no lane file of its own; a repo with no `.bee/lanes/` at all places its one feature correctly; dropped cells count nowhere (D8) |
| **E3 — What needs attention** | D6's generated rules, and the review/debt readers that feed them | This is what makes it a management view rather than a data dump | S1a, S3 | Each rule fires and stays silent under fixtures built for it; a fixture tripping several rules at once orders them heaviest-first; every emitted item carries a suggested action; an empty list says so in one line |
| **E4 — Nothing lost** | D2's velocity, worktrees, workspaces; process health | The redesign must not cost signal already earned | S4 | Every number the old board showed is still reachable on the new one |
| **E5 — The page holds up** | Responsive, both themes, keyboard, and the test sweep | The invariants must survive the markup they were written against | S5 | No horizontal page scroll at 375px; theme attribute beats OS both directions; every bucket-B test re-expressed or explicitly retired |

**Slice queue**

| Slice | Contents | Depends on |
|---|---|---|
| **S1a** (current) | The free-text scrubber and its tests (D9). `snapshot_tree` learns to record directories, with all six existing read-only tests green over the change. Reader learns `approved_gates`, `gate_revoked_at`, `route` and `next_action` — all from `state.json`, a file it already opens, so no new reader and no new fixture builder. The full D5 skeleton: header, lifecycle stepper (D7-correct), KPI row, "working on now" card, attention panel carrying the rules existing snapshot data already supports (blocked cells, read errors). The old sections remain below, untouched. | — |
| **S1b** | The first two new file readers: `.bee/HANDOFF.json` and `.bee/config.json`, each with its fixture builder **and its own read-only fixture in which the file is present and populated**. Attention rules extended with the stale-handoff and bypass-not-off rules. | S1a |
| **S2** | Lane records read with their own phase/gates/created_at (`lane_json` must grow both fields first); per-feature cell counts; the by-phase board replaces **both** the four-bucket section and the existing lanes panel (`bee_lanes_panel`, `views.rs:1357`), which otherwise renders the same features a second time; cards link to detail pages (D3). | S1a |
| **S3** | Review join (`review-candidates.jsonl` × `.bee/reviews/*.json`), capture queue, scribing-debt derivation, promote-proposal presence. Attention rules extended to open P1s, unreviewed high-risk, knowledge debt. Review/backlog panel. | S1a |
| **S4** | Velocity folded in as the delivery-speed block; worktrees and workspaces folded into the sessions panel; the old running-now section (`bee_running_now_section`, `views.rs:739`) retired into S1a's "working on now" card, which by then carries its whole job; process-health panel (tier mix, reservations, gate bypass, read errors). | S1a, S2 |
| **S5** | The cross-cutting pass only: responsive, both themes, keyboard and focus, and re-syncing `docs/specs/bee-cockpit.md` to the board that now exists. | S2, S3, S4 |

**Each slice re-expresses the tests it breaks, inside that slice.** The 28 markup tests split by section — the finished-work tests to S2, the panel tests to S3, the running and worktree tests to S4 — so no slice ships red and no slice leaves a rule for a later sweep to remember. This is the smaller path found at the gate: an earlier draft held a big-bang test sweep as its own final slice, which would have meant the middle of the feature ran with rules deliberately unproven and one slice carrying the whole risk. Two of the 28 do not split cleanly — `panels_render_backlog_sessions_and_lanes_with_liveness` (`server.rs:2908`) and `absent_backlog_sessions_and_lanes_render_honest_empty_states` (`server.rs:2993`) each assert across backlog, sessions and lanes in one body. They are split into per-area tests in S2, the first slice that touches any of the three, rather than being edited three times.

**Current slice to prepare: S1a only.** It is a walking skeleton by construction: real store data, real page, no stubs, and the old sections keep working beneath it, so the page is never half-built in the browser.

**S1a's proof is not only its own new tests.** Three existing tests assert over the *whole* response body, so S1a's markup falls inside their scope even though S1a changes none of their sections. S1a is not green until all three still pass, and each names a way the new sections could break them:

- `no_shipped_features_renders_honest_empty_state_not_zeros` (`server.rs:2731`) forbids the substrings `0.0` and `0/0` anywhere in the body. Its fixture has two cells, nothing shipped, and **no `state.json`** — precisely the state where a naive progress bar emits `width: 0.0%` or a KPI reads `0/0`. S1a's honest-empty-state handling is what this test measures.
- `board_renders_finished_work_in_exactly_one_place` (`server.rs:3434`) asserts a feature name appears exactly twice. A stepper or "working on now" card that falls back to cell data when `state.json` is absent would name it a third time.
- `detail_pages_leak_no_absolute_path_or_fixture_root` (`server.rs:3634`) forbids a fixture path in the body.

## Rules the current tests carry (must survive the markup)

The 92 tests over this area split four ways: **12 invariants** that must survive verbatim (no absolute path, no transcript path, the `.bee` tree byte-identical after every request, 404 for a project with no `.bee`), **7** that only touch the two detail pages and must keep passing untouched, **45** reader-level tests in `bee.rs` that never see markup, and **28** that assert board markup and will break.

Those 28 are not all wording. Fourteen of them encode a behavioural rule the redesign must still honour even though every string around it changes. Losing one of these silently is the specific failure this plan exists to prevent, so each is listed with the test that currently holds it and must be re-expressed against the new markup:

| Rule | Currently held by |
|---|---|
| A finished feature is rendered exactly once on the board, never twice | `board_renders_finished_work_in_exactly_one_place` (server.rs:3434) |
| Finished work loads collapsed, not expanded | `board_done_details_element_has_no_open_attribute` (server.rs:3405) |
| Nothing to measure renders an honest empty state — never `NaN`, `Infinity`, `0.0` or `0/0` | `no_shipped_features_renders_honest_empty_state_not_zeros` (server.rs:2731) |
| An empty finished-work section is a plain line, not a zeroed collapsible list | `board_done_section_renders_honest_empty_state_when_nothing_done` (server.rs:3531) |
| Finished cells group one compact line per feature — never one card per cell | `board_done_section_groups_by_feature_and_states_true_total` (server.rs:3299) |
| The finished list is uncapped — no feature is silently dropped | `board_done_section_shows_every_finished_feature_uncapped` (server.rs:3333) |
| Any capped or truncated list states its true total beside the visible subset | `capped_findings_subset_states_its_true_total` (server.rs:2965) |
| Bucket/phase membership is a pure function of cell status — live-worker data never re-places a cell | `d7_buckets_unchanged_by_worker_presence` (server.rs:3827) |
| A granted worktree's own cells never merge into this project's counts, and its cell ids never render here | `worktree_cell_files_do_not_change_buckets_or_shipped_set` (server.rs:4132) |
| A live-worker/store disagreement is stated explicitly, never hidden | `running_worker_on_still_open_cell_shows_discrepancy_note` (server.rs:3795) |
| A worker naming an unknown cell is flagged, never silently dropped, and the page still renders | `worker_naming_nonexistent_cell_is_flagged_and_page_still_renders` (server.rs:3885) |
| A worker whose session has gone stale is never presented as running | `worker_with_stale_session_not_presented_as_running` (server.rs:3916) |
| A resolution failure degrades to "unresolved", never to a page-level failure | `worktree_directory_missing_is_unresolved_and_page_still_renders` (server.rs:4205), `worktree_state_json_malformed_is_unresolved_not_fatal` (server.rs:4225) |
| Per-cell file lists live only on the cell detail page, never on the board | `board_card_drops_file_list_but_cell_detail_page_keeps_it` (server.rs:3560) |

Read together these are one principle the board already holds and the redesign inherits: **the page never manufactures a number it does not have, and never quietly drops something it cannot explain.** D5's honest-empty-state rule and D6's attention list are the same instinct carried further.

The fixture family covers the shape of this work but not its content: `fresh_root` (server.rs:2306) plus `write` (server.rs:2317) build a fake `.bee` tree file by file, with a JSON builder per record type. `lane_json` (server.rs:2871) exists but emits only `feature`, `phase`, `mode` and `next_action` — it carries neither `approved_gates` nor `created_at`, the two fields S2 exists to read, so it must grow before S2 can be tested. A handoff, a config file, a capture queue, review candidates and reservations each need a new reader in `bee.rs` and a matching builder; their on-disk shapes are recorded in Discovery above, read from the two live stores.

**Reviewed twice before the gate.** A plan-check pass verified every anchor and Discovery claim against the repo and found one P1 — the D9 free-text gap now written into Approach, the risk map and the test matrix — plus two P2s (the duplicate lanes panel and running-now section nothing had retired; the three whole-body tests S1a must keep green) and eight P3 corrections. Its confirmations are load-bearing too: the test taxonomy (12 invariants / 7 detail-page / 28 markup / 45 reader) and all fifteen rule anchors in the table above were verified individually.

A separate high-risk advisor consult then judged the shape rather than the anchors, confirmed high-risk as the honest lane, and returned **proceed with four conditions**, all now written into the plan above rather than left as advice:

1. Every new reader ships with its own read-only fixture in which its file is **present and populated** — the existing six read-only tests each use a fixture containing only their own section's files, so a new reader would take its missing-file early return and never run the code that could write.
2. `snapshot_tree` records directories as well as files, in S1a, with all six existing read-only tests green over the change — otherwise a `create_dir_all` before a `read_dir` writes into the user's store while the assertion passes.
3. The `gate_bypass` resolution rule is settled before it is coded: the board renders what `config.json` literally records and labels it as such, because `.bee/config.local.json` is a machine-local overlay and a wrong effective-level claim is worse than no claim.
4. Any feature-name→path join is validated at the join site before it touches the filesystem — no separators, no `..`, not absolute — with a traversal-shaped `feature` probe. This binds S3 but is settled now, because the first person to write that join sets the pattern.

Its recommended split of S1 into S1a and S1b was taken: S1b holds the only part that opens new files, so a red there is unambiguous and the read-only proof for two new readers does not ride in as a passenger on a markup rewrite.

## Test matrix

High-risk lane — probes per applicable dimension. Each cell's writer judges existing coverage first and authors only the gap; several dimensions are already pinned and must simply not regress.

| Dimension | Probe | Status |
|---|---|---|
| Empty / absent | No `.bee/lanes/`; no `.bee/reviews/`; no `HANDOFF.json`; empty `backlog.jsonl`; a feature with zero cells | New — this repo's store is the zero-lanes fixture |
| Malformed input | A lane file that is not JSON; a review session with no `decision`; a finding with no `severity` key and one whose severity is `info` (both live in beehive today, alongside 17 P1 / 18 P2 / 38 P3); a review candidate naming zero cells with a null baseline (this repo's only candidate); a cell with no `tier` | Convention exists (`read_errors`), new cases needed |
| Boundary | Exactly one feature; a feature whose cells are all `dropped` (D8 — the denominator must not divide by zero); tier mix with zero tiered cells | New |
| Untrusted content | A cell title containing `<`, `>`, `&`, `"`; a handoff `next_action` containing markup | `esc()` exists at `views.rs:2213`; needs a case at each new render site |
| Information disclosure | An absolute path embedded mid-sentence in each new free-text field — `next_action`, `route.rationale`, handoff `next_action`, a review finding's prose — plus direct unit tests of the scrubber | **New.** The existing leak tests cover only wholly-path fields and would pass while a handoff leaked a home directory. They must also keep passing. |
| Side effects | Every new reader gets its own read-only fixture **with its file present and populated**, in the slice that introduces it | **New.** The six existing read-only tests each use a fixture containing only the files their own section reads; none contains a handoff, a config, a capture queue, review candidates, a review session or reservations, so every new reader would take its missing-file early return and the tests would pass without ever running the code that could write. |
| Side effects — the probe itself | `snapshot_tree` records directories as well as files, and every existing read-only test stays green over the change | **New.** `snapshot_tree` (`server.rs:2414-2432`) pushes an entry per file and none per directory, so a `create_dir_all` before a `read_dir` — the obvious idiom for listing `.bee/reviews/*.json` — writes into the user's store while the assertion passes. |
| Path construction | A cell and a `state.json` carrying a traversal-shaped `feature` (`../../..`, an absolute path, a separator) | **New, binds S3, decided now.** `docs/history/<feature>/promote-proposals.md` is this area's first join from a store string to a filesystem path; `feature` is unvalidated free text everywhere it is read (`bee.rs:680-684`), and the nearest precedent (`resolve_worktree`, `bee.rs:1073`) joins an unvalidated key too, so house style would copy the hole. The slug is validated at the join site — no separators, no `..`, not absolute — before it touches the filesystem. Presence-rendering alone would otherwise make the board a filesystem oracle for paths outside a project mdview does not own. |
| Presence rule | A registered project with no `.bee` is 404, not an empty board | **Already pinned — must not regress** |
| Concurrency / staleness | A session whose heartbeat is minutes old vs an hour old; a worker naming a cell that no longer exists | Pattern exists (`SESSION_LIVE_MINUTES`, `compute_running_workers`); extend, do not re-invent |
| Version drift | `route` carrying `feature` (2.2.2) vs `demoted_at` (2.1.15); `gate_bypass` as `false` vs `"total"` | New |
| Rendering / a11y | No horizontal page scroll at 375px; `data-theme` beats `prefers-color-scheme` in both directions; semantic colour never alone | New, S5 |

Every cell caps through `bee finish`, which runs the one declared command, `cargo test --workspace` — the same command close, merge and CI re-run.

## Out of scope

- Live auto-refresh (SSE, file watching). Deferred in CONTEXT.md: it turns the page from a document into a connection and needs its own look at the read-only guarantee.
- A cross-project roll-up. This feature is per-project by definition.
- Any change to the two detail pages beyond keeping their links working (D3).
- Executing the bee CLI to fill a gap the files cannot (D4). Where the files genuinely cannot answer, the board says so rather than guessing.
