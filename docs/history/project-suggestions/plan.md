---
artifact_contract: bee-plan/v1
mode: high-risk
approved_gate2: 2026-08-08
---

# Plan: Project Suggestions

Mode: `high-risk` — 2 risk flags: audit-security (full filesystem paths rendered on an
unauthenticated, LAN-reachable page; partial supersession of web-interface R5), changes
behavior an existing test asserts (`unauthenticated_home_page_shows_unassigned_presence_only_and_leaks_nothing`
pins that no stray pane's cwd appears on `/`).
Why this is the least workflow that protects the work: the code is small, but it
deliberately relaxes a recorded information-exposure rule, so the shape and its test
re-expressions need independent review before any write. (Review wave ran: coherence,
feasibility, security — findings folded in below.)

## Requirements (from CONTEXT.md)

- D1: Suggestions come from herdr — every session whose folder sits under no registered
  project becomes a suggestion pointing at that pane's own working directory, exactly as
  reported, no walk up to a repository root.
- D2: The suggestion block shows full filesystem paths; registered project rows still
  never do. (Supersedes web-interface R5 in part.)
- D3: The block is gated on `terminal.enabled` alone, not additionally on
  `unassigned_enabled`. (Narrows toa-4/D9 for one surface.)
- D4: One suggestion row per distinct unregistered cwd, with a count of agent panes
  running inside it — never one row per pane.
- D5: Each suggestion's Register button posts the path to the existing
  `/api/projects/register` route; failures reuse the `register_error` banner codes.
- D6: Only agent-backed panes produce suggestions, matching the Unassigned group's
  computation; shell-only panes never do.
- D7: Stateless — recomputed each render, no dismiss, no persistence.

## Discovery

Inspected `index_page`/`project_list_page` (server.rs:343-405, views.rs:91), the
complement computation `unassigned_panes` (server.rs:1861-1904), the register route and
its validation (server.rs:716-831), the boundary rules
(mdview-core/src/paths_boundary.rs:135-155, 241-246), and the leak-nothing home-page
test (server.rs:10222-10286). The review wave verified every anchor against the code;
`cargo test --workspace` is the declared suite and the leak test is green at baseline.
Access model confirmed from terminal-open-access D1/D2: the app has no authentication
anywhere; `terminal.enabled` is the only gate, and the owner has accepted that exposure.

### Planning answers to CONTEXT's deferred questions

- **cwd vs `foreground_cwd`:** suggestions read `Pane.cwd` only, mirroring
  `unassigned_panes` (server.rs:1886-1891) per D6. A unix pane reporting only
  `foreground_cwd` yields no suggestion. Membership (who is assigned) stays with
  `project_panes`' cwd-then-`foreground_cwd` fallback (server.rs:1797-1805),
  untouched. This is display-side only; either choice preserves the membership
  invariant (security note N6).
- **Row ordering:** suggestion rows sort by path, ascending, byte-wise. Deterministic,
  no new comparator logic.

## Approach

Recommended path (cites D1–D7): compute suggestions inside `index_page`, from the SAME
single herdr snapshot it already takes under `terminal_family_enabled`
(server.rs:352-363, 2s timeout; timeout/error renders the page without the block).
The suggestion source is the `unassigned_panes` complement (per D6) — a new call site;
today its only callers are the `/_terminal/unassigned` routes. **Prohibition: the
assigned set is never derived from `with_counts`' per-row fail-open badge path
(server.rs:368-383) — only from `unassigned_panes`' whole-group fail-closed rule
(server.rs:1871-1878): one unconstructable project boundary means zero suggestions.**
The call lands before `projects.into_iter()` consumes the list (server.rs:364).

Group the complement's panes by cwd (per D1, D4): normalize only by trimming a
trailing slash (never canonicalize, per D1's "exactly as reported"), drop empty cwd,
count panes per folder. **Second guard (from security review B1): drop any suggestion
whose normalized cwd equals or sits lexically under a registered project's root** —
`validate_existing` refuses for non-containment reasons too (deleted dir, `..` in the
raw string, symlink loop, EACCES; paths_boundary.rs:135-155), and without this guard
such a pane would surface a path *inside* a registered project on `/`, violating D2's
unsuperseded half. The guard only ever drops rows — its failure direction is safe.

Render via a dedicated row type carrying **path and count only** — never
`TerminalPaneView`, whose `name`/`title`/`workspace`/`tab` fields D2 does not
authorize on this page. `project_list_page` (views.rs:91; exactly one caller,
server.rs:396) grows one parameter. The block is its own section styled after the
card pattern (views.rs:169-179) — visual template only: that card is an `<a>` wrapper
and a form cannot nest inside an anchor (the repo pins this class of nesting,
server.rs:~10153), so each suggestion row is its own element containing the path
text, the count, and a `<form method="post" action="/api/projects/register">` with
the path as a hidden input (per D5; pattern views.rs:211-219). The visible path text
and the hidden input value are byte-identical. All output through `esc`
(views.rs:3306; escapes `& < > "` — attributes stay double-quoted, the file's
convention). The form carries none of the delete-confirm classes (`proj-row__delete`)
so app.js never attaches to it. The stale rationale comment at server.rs:387-394
(written when the marker was the only disclosure) is corrected in the same cell.

Register flow: the route redirects to `/?register_error={code}` on refusal and `/` on
success (server.rs:749-750), so a failed one-click register lands back on the page
with the existing banner (views.rs:184-190, 229-241), and a successful one removes
the row because the folder is now registered (D7). A suggestion whose path is
deny-listed (e.g. `$HOME` — routine for stray shells) still shows and fails with
`denied` on click: show-and-fail is deliberate; filtering would require duplicating
the deny list, which server.rs:768-771 explicitly refuses (security note N2).

CSRF posture, considered and accepted (security note N1): register is already an
unauthenticated form-encoded POST reachable cross-site; prefilling the path changes
shape (N one-click forms on `/`), not authority, and the same one-click pattern
already ships on this page as unregister. toa-D10's JSON-preflight reasoning protected
the settings endpoint; applying it here would break D5's "route unchanged" and the
no-JS constraint.

Rejected alternatives:
- New JSON endpoint + client fetch — the page is synchronous server-rendered today; a
  fetch layer adds surface for zero benefit (D7).
- Walking up to a repo root before suggesting — explicitly forbidden by D1.
- Gating on `unassigned_enabled` too — explicitly forbidden by D3.
- Filtering deny-listed suggestions — duplicates the deny list (refused at
  server.rs:768-771); show-and-fail instead.

Compact risk map:

| Component | Risk | Reason | Proof needed |
|---|---|---|---|
| Path display on `/` | HIGH | Relaxes R5's exposure rule on an unauthenticated page | Named test re-expressions (below); switch-off phase byte-identical; ON-state pins for every unauthorized field |
| Owned-folder suggestion | HIGH | `validate_existing` refuses on non-containment grounds; without the lexical second guard an owned subtree path leaks | Deleted-subfolder probe: registered P, pane cwd = P/gone → no suggestion |
| HTML injection via cwd | MEDIUM | cwd is attacker-influencable text rendered into HTML text and a form attribute | Hostile path containing `"` and `'`; assert escaped output and double-quoted attribute |
| Fail-open inversion | MEDIUM | Deriving assigned set from `with_counts`' fail-open path would publish an unconstructable project's own panes | Two projects, one unconstructable root → zero suggestions overall |
| Register flow | LOW | Route reused unchanged (D5) | Duplicate/denied posts surface existing banner codes |

## Shape

Epic map — one epic, one slice; the work is genuinely one slice and is not forced
into phases.

| Epic | Capability/Risk Area | Why It Exists | Slices | Proof Needed |
|---|---|---|---|---|
| Suggestion block | Surface unregistered working folders on `/` with one-click register | D1–D7; closes the gap where running agents sit in folders the dashboard cannot see | 1 | `cargo test --workspace` green including the named leak-test re-expressions |

Slice 1 (walking skeleton — end-to-end, real behavior, no stubs), two cells; each
cell's exit is `cargo test --workspace` green at `bee cells finish`, and both cells
carry the prohibition **register route logic untouched** (`register_project`,
`validate_register_path`, the deny list, and the bounded scan are read-only).

- **ps-1 — Compute and render the suggestion block.** `crates/mdview/src/server.rs`,
  `crates/mdview/src/views.rs`. Everything in Approach up to the register flow:
  complement reuse with the fail-open prohibition, cwd-only grouping with
  trailing-slash normalization and the owned-subtree second guard, path+count row
  type, D3 gate, sorted-by-path rows, escaping, comment fix.

  **The leak-test surgery, named assertion by assertion**
  (`unauthenticated_home_page_shows_unassigned_presence_only_and_leaks_nothing`,
  server.rs:10222-10286 — three phases):
  - Phase 1 (switch off, :10236-10246): stays **byte-identical**. Off = no herdr
    call = no block; already proven at baseline.
  - Phase 2 (family-only, :10248-10260): the whole-body
    `!body.contains("Unassigned"|"unassigned")` assertion **cannot survive as
    written for a second reason the wording rules don't fix: the fixture path
    itself** (`fresh_root("home-unassigned-presence-scratch")` →
    `.../mdview-server-bee-home-unassigned-presence-scratch-<pid>-29/stray`)
    contains the substring `unassigned`, and per D3 the block now renders that path
    in this phase. Re-express it to target the marker itself: assert the body
    contains neither `"Unassigned agents"` nor `href="/_terminal/unassigned"`,
    AND assert the suggestion block with the stray path IS present (the narrowed
    toa-4/D9 pin plus the D3 pin, in one phase). Reason recorded: D3 supersession +
    fixture-substring collision.
  - Phase 3 (both on, :10265-10286): `!body_on.contains(&stray_root...)`
    (:10279-10282) is **the one assertion whose truth value D2 inverts** — it
    becomes `assert!(body_on.contains(...))`, reason recorded as the D2
    supersession. `!body_on.contains(&stray.name)` (:10275-10278) **survives
    unchanged** — D2 authorizes path + count, never the agent's name. Extend it to
    the full unauthorized field set: with the block rendered, no agent `name`,
    `title`, workspace label, tab label, or `pane_id` appears anywhere in the body.

  New tests (triaged from the matrix): grouping/count + trailing-slash merge,
  D3 gating, fail-closed (two projects, one unconstructable root → zero
  suggestions), owned-subtree guard (deleted subfolder of a registered root → no
  suggestion), escaping (hostile path with `"` and `'`; text equals hidden value),
  empty-state (no suggestions → no block markup), no-cwd pane produces nothing,
  boundary triple (cwd equal to root = assigned; child = assigned; sibling with
  shared prefix = suggested), snapshot timeout renders page without block.
- **ps-2 — Register button per suggestion.** Same files, deps ps-1. The register-flow
  paragraph of Approach: per-suggestion form posting the exact suggested path; on
  success the row disappears (D7); on failure the existing banner shows. Tests:
  happy register from a suggestion (row gone after redirect), duplicate
  (double-click) surfaces `duplicate`, denied root surfaces `denied` (show-and-fail
  pinned), and the D2-second-half ON-state pin: with the terminal switch on and a
  registered project present, the registered row still shows no path — the existing
  pin at server.rs:9597-9600 is switch-off-scoped and stays; this cell authors its
  ON-state analogue.

## Test matrix

High-risk: probes per applicable edge dimension, each mapped to a cell's truths;
writers judge existing coverage first (the register route's own matrix in
projects-home plan.md S2 already pins validation and stays untouched).

| Dimension | Probe | Cell |
|---|---|---|
| 1 User types | Anyone reaching `/` sees suggestions only when `terminal.enabled` is on; off = zero trace (phase 1 byte-identical) | ps-1 |
| 2 Input extremes | Pane with no cwd (or only `foreground_cwd`) produces no suggestion; hostile path with `"` and `'` renders escaped in text and double-quoted attribute; text and hidden value byte-identical | ps-1 |
| 3 Timing | herdr snapshot timeout/error renders the page without the block (bounded, matches R6's plain-rows rule) | ps-1 |
| 4 Scale | 0 suggestions = no block markup at all; several panes in one folder = one row, correct count; `/a/b` and `/a/b/` merge | ps-1 |
| 5 State transitions | Register clicked twice → second post surfaces `duplicate` banner, no state damage | ps-2 |
| 8 Authorization | None by design (toa-D1); the gate is D3's switch — pinned by the dimension-1 probes | ps-1 |
| 9 Data integrity | Register route untouched (prohibition in both cells); owned folder never suggested: equal-root/child/sibling triple + fail-closed probe + deleted-subfolder second-guard probe | ps-1 |
| 11 Compliance | Switch off: nothing anywhere on `/`; family-only: block with path present, Unassigned marker absent; both-on: path present, agent name/title/workspace/tab/pane_id all absent; registered rows show no path with the switch on | ps-1, ps-2 |
| 12 Business logic | Boundary values: cwd equal to a project root = assigned; direct child = assigned; sibling with shared prefix = suggested | ps-1 |

Dimensions 6, 7, 10 (environment, error cascades beyond the herdr timeout, external
contract drift) add nothing here: no new dependency, no new external call, no schema.

## Out of scope

- Any change to the register route's validation, deny-list, or bounded scan.
- The Unassigned group's own switch, routes, or wording (toa-4/D9 stands except as
  narrowed by D3 for this one surface).
- Repo-root inference, dismissal state, client-side polling (D1, D7 forbid).
- Spec sync for web-interface.md R5's partial supersession lands via scribing at
  close, not as a product cell.
