---
artifact_contract: bee-plan/v1
mode: standard
---

# Plan: Kanban Columns

Mode: `standard` — 2 risk flags: covered-contract-change, proof-weakening
Why this is the least workflow that protects the work: the board's three-column
shape is pinned by a documented contract and a wall of existing tests, so the
shape needs one written, reviewed pass — but every byte of data the five columns
need already exists in the snapshot, so nothing below the view layer moves.

## Requirements (from CONTEXT.md)

- **D1** — five columns, left to right: Todo, In Progress, Review, Compound, Finished. The "Waiting on you" column is removed.
- **D2** — Todo holds unclaimed-open features first, then `proposed` PBIs.
- **D3** — a PBI renders as a dense flat row (Finished's row shape), clickable to its project's bee board.
- **D4** — Review = unresolved review candidate; Compound = lane `phase == "compounding"`; Finished keeps its current rule.
- **D5** — In Progress wins every tie: live work beats a waiting candidate, and the active feature never falls to Todo.
- **D6** — superseded by D12.
- **D7** — a gate-stopped feature stays in In Progress with one card line: `Waiting on you — gate <name>`.
- **D8** — that reason comes from the existing `bee_gate_current_stop` rule (and the pause-handoff case), carried over unchanged.
- **D9** — both boards get the five columns: the cross-project home board and each project's own board, sharing one classification rule.
- **D10** — live work narrows for placement: `doing`/`stuck` cells plus the active/session/worktree pulls; an `open` cell alone is not live.
- **D11** — placement order: In Progress, Finished, Review, Compound, Todo; a closed feature stays Finished even with an unresolved candidate.
- **D12** — Todo, Review, Compound, Finished render dense one-line rows and page at ten, exactly as Finished does today; only In Progress renders cards.

## Discovery

Every input the new columns need is already on `BeeSnapshot`; no collector or
core type changes. `snapshot.review.candidates` is a `Vec<BeeReviewCandidate>`
carrying `feature` and a `BeeReviewStatus` of `Unreviewed | InReview | Settled`
(`crates/waggledance-core/src/bee.rs:719,733,695`). `snapshot.backlog.pbis` is a
`Vec<BeePbi>` with `status: String` — `"proposed"` among its documented values —
and a `feature` field (`bee.rs:394,453`). "Open with none claimed" is derivable
from the counts already on `BeeFeaturePhase`: `waiting > 0 && doing == 0`
(`bee.rs:646`). Project-scoped hrefs follow the existing
`/p/{pid}/_bee/feature/{feature}` shape (`views.rs:2972`).

Two sections render the three groups, both from the shared classifier:
`bee_feature_hub_section` (per-project board, `views.rs:2338`) and
`bee_cross_project_features_section` (home page, `views.rs:2468`). Both grow to
five columns (per D9).

The existing proof is heavy: 43 test functions pin the three-group shape —
11 on the Waiting group, 7 on In Progress, 15 on Finished, 7 on the group
count and section shape, 5 fragile by association. Two of them state the
three-group contract in words rather than data:
`feature_hub_empty_store_renders_three_honest_empty_groups`
(`server.rs:4827`) loops the three keys, and
`board_no_longer_declares_a_wide_scrolling_container` (`server.rs:17161`)
asserts "the new grouped list is only ever three groups". Both must be
rewritten to the five-column contract, not deleted. The whole Waiting group —
Group A's 11 tests — is rewritten to assert the same features now land in
In Progress carrying the D7 line; that is where the `proof-weakening` flag
actually bites, and no assertion in it may simply disappear.

Consequence: this feature is a single-file change to
`crates/waggledance/src/views.rs` — its classifier, its two section renderers,
its group helper, its CSS block, and its own test module — plus the board and
home-page tests in `crates/waggledance/src/server.rs`.

## Approach

Extend `BeeHubPlacement` (`views.rs:2174`) from three variants to five plus a
PBI row, rewrite the if-chain in `bee_classify_features` (`views.rs:2212`) so
In Progress absorbs the former Waiting pull, and render four of the five
columns through the dense row and pager that already serve Finished (per D12) —
so the column shape stays exactly one card column beside four row columns.

The chain's first move is D10: `live` at `views.rs:2231` stops counting
`waiting`, so `has_live_work` becomes `doing + stuck > 0 || is_active ||
session_bound || worktree_bound`. Without that narrowing every feature Todo is
meant to hold is swallowed by In Progress first and D2's column is dead code.
`finished_and_idle` keeps its own definition of idle — no live *cells* at
all, `doing + waiting + stuck == 0` — so today's Finished behaviour is
preserved exactly.

Placement is then tested in this order (per D11):

1. **In Progress** — `has_live_work && !finished_and_idle`, with `has_live_work` as narrowed by D10. This swallows the whole former Waiting branch (per D5, D7), so `working_now` stops gating placement and only feeds the card line.
2. **Finished** — `is_finished` (`phase == "compounding-complete"` or an archived-cells directory), unchanged from today (per D4).
3. **Review** — an unresolved candidate names this feature: `snapshot.review.candidates` holds a `BeeReviewCandidate` whose `feature` matches and whose `status` is not `Settled` (`Unreviewed` or `InReview`).
4. **Compound** — lane `phase == "compounding"` (per D4).
5. **Todo** — `waiting > 0 && doing == 0` (per D2).

Finished before Review is D11's deliberate call: a closed feature stays closed.
Review before Compound puts a feature carrying both signals at its earlier
stop. Anything matching none of the five still renders nowhere, exactly as
today.

`bee_hub_finished_row` (`views.rs:2946`) becomes the shared row for all four
dense columns, and `bee_hub_finished_rows` / `bee_hub_finished_more`
(`views.rs:2989`, `views.rs:3005`) become their shared pager — both stay, and
their existing tests stay with them. The row today hardcodes
`data-hub-group="finished"` and a feature-detail href (`views.rs:2972`), so the
group key and the href both become parameters; every existing Finished
assertion must still pass with `finished` passed in.

PBIs join Todo below the features (per D2), through that same row. A PBI row
links to its project's bee board, `/p/{pid}/_bee` (per D3) — not to a feature
page, which a `proposed` PBI generally does not have.

On the cross-project board the merge needs two accumulators per column, not
one: features from every project first, then PBI rows from every project.
Appending both into one Todo string in placement order would put project A's
PBIs above project B's features and break D2's ordering.

Rejected alternatives:

- Keep Waiting as a sixth column — rejected by D1; it splits running work across two places.
- Heading-and-count-only tail columns — was D6, superseded by D12: a count alone is unreadable.
- A second row renderer for the new columns — rejected; Finished's row and pager already do exactly this job and carry their own tests.
- A new `BeeSnapshot` field for review/PBI data — rejected: the data is already there (see Discovery).
- Keeping `waiting` cells inside `has_live_work` — rejected by D10; it makes Todo unreachable.

Three named consequences the writer must carry, not discover:

- **The `Waiting on you` line's exact wording.** `bee_gate_current_stop` returns a display label, not a key (`("shape", "Shape")`, `views.rs:2651`), and today's caller formats `"{label} gate awaiting your decision"` (`views.rs:2275`). The line reads `Waiting on you — Shape gate awaiting your decision` for a gate stop and `Waiting on you — waiting on your decision` for a pause handoff, reusing today's two strings verbatim behind the new label.
- **Every column keeps its empty-state line.** `bee_hub_group`'s doc (`views.rs:2673`) cites bee-board-pm D5, "sections never disappear", and emits an `fg-empty` line at zero count. That contract is untouched — the two new columns each need their own empty-state wording, and Waiting's ("Nothing waiting on you.") goes away with its column.
- **The grid carries four row columns and one card column.** `repeat(auto-fit, minmax(260px, 1fr))` (`views.rs:1688`) was sized for three equal columns; five equal 260px tracks overflow a laptop. In Progress needs the card width, the four row columns do not. The narrow-screen collapse to `grid-template-columns: 1fr` (`server.rs:17000`) must keep working.

Doc comments that become false and must be rewritten with the code, not left
behind: `views.rs:1503` ("exactly one of three groups"), the long three-group
contract at `views.rs:2041-2115` including the whole Waiting rule at
`views.rs:2066`, `views.rs:2154`, `2170`, `2201`, `2326`, `2429`, and the
`group_key` note at `views.rs:2732`.

Risk map:

| Component | Risk | Reason | Proof needed |
|---|---|---|---|
| `bee_classify_features` | MEDIUM | Five branches replace three, and the Waiting pull is folded into In Progress — a placement regression is invisible until someone reads the board | Unit tests per branch and per tie case, in the existing test module |
| Existing board tests | HIGH | 43 tests pin the three-group shape, 11 of them on a group that ceases to exist | Every Waiting assertion rewritten to pin the same feature in In Progress with its D7 line — never simply dropped; the two word-stated three-group contracts restated for five |
| Five-column CSS | LOW | `repeat(auto-fit, minmax(260px, 1fr))` sized for three columns will wrap five | Explicit grid tracks: one card column, four narrower row columns; the narrow-screen `1fr` collapse still asserted |
| Card `Waiting on you` line | LOW | Reuses `bee_gate_current_stop` untouched (per D8) | One test per reason source (gate stop, pause handoff) |

## Shape

Two cells, sequential, split by data source rather than by layer.

Cell 1 carries the whole enum change because `BeeHubPlacement` is matched
exhaustively by both section renderers (`views.rs:2345`, `views.rs:2510`) —
Rust forces both call sites into the same edit, and any smaller step would
leave the crate red or add fold-back scaffolding written only to be deleted.
The `Waiting on you` line rides along in cell 1 rather than waiting for a
second pass: the reason string is already computed at `views.rs:2274`, so
carrying it into the In Progress branch costs cell 1 nothing, and deferring it
would ship a state where the board silently loses its gate reasons.

Cell 2 is genuinely independent — PBIs are a different data source
(`snapshot.backlog.pbis`), a different renderer, and their own tests — and it
leaves cell 1's board green and complete on its own.

| Phase | What Changes | Why Now | Demo | Unlocks |
|---|---|---|---|---|
| 1 — Five columns for features | `has_live_work` narrows per D10; `BeeHubPlacement` grows to five variants; the classifier chain is rewritten per D11; the dense row and pager are parameterized and wired into all four row columns per D12; both section renderers, the grid tracks, and the `Waiting on you` card line land; the false doc comments go | Nothing can be demonstrated until the boards themselves have five columns, and the gate reason must not disappear on the way | Both the Kanban tab and a project board show Todo, In Progress, Review, Compound, Finished — four as paged row lists, In Progress as cards, a gate-stopped feature carrying its line | Cell 2 |
| 2 — PBIs into Todo | `proposed` PBIs render as flat rows below Todo's features, linked to their project's bee board, with the cross-project merge keeping features above rows | Todo exists and is populated by cell 1; PBIs are a separate source that can land without touching placement | A `proposed` PBI shows as a linked row under Todo on both boards | — |

## Test matrix

Each cell's writer judges existing coverage first and rewrites the assertions
the change invalidates rather than deleting them.

**Happy path** — a feature with a claimed cell lands in In Progress; a feature
with only open cells lands in Todo; a `proposed` PBI renders a linked flat row
under Todo; a `compounding` feature lands in Compound; a feature with an
`Unreviewed` candidate lands in Review; a `compounding-complete` feature lands
in Finished; the section renders exactly five groups in D1's order.

**Edge cases** — live work plus an unresolved candidate goes to In Progress, not
Review (D5); the active feature with every cell open goes to In Progress, not
Todo (D5); a non-active feature with every cell open goes to Todo, which is the
D10 narrowing's own proof; a closed feature with an unresolved candidate stays
in Finished (D11); a feature carrying both an unresolved candidate and
`compounding` goes to Review; a `Settled` candidate does not pull a feature into
Review; a feature matching no branch renders in no column; Todo, Review, and
Compound each page at ten rows with the remainder behind nested disclosures,
and render no card markup at any count (D12); a gate-stopped feature shows
exactly one `Waiting on you` line with its exact string, a pause-handoff feature
shows the pause wording, and a feature worked on right now shows none; on the
cross-project board, PBI rows from every project sit below the features of every
project (D2).

**Error paths** — an empty snapshot renders all five headings with zero counts
and each column's own empty-state line; a PBI whose `feature` names no
lane or feature page still renders its row with a working project-board href; a
feature stopped on the `review` gate is excluded from the line's reason, as
`bee_gate_current_stop` already excludes it (D8).

## Out of scope

- `bee_finished_section` — the board's separate bottom shipped-features `<details>` list, which the code's own comments (`server.rs:6891`) already keep distinct from the hub's groups — and the Projects tab. Both untouched.
- Remembering a viewer's expand choice — the paging disclosures are the same stateless native ones Finished uses today (D12); nothing is remembered.
- Any change to `waggledance-core`'s snapshot types or collectors.
