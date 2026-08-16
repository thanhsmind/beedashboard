# Backlog Groom 2 — Context (views.rs cluster)

**Feature slug:** backlog-groom-2 · 2026-08-16 · standard · bugfix batch

## Scope

Five review findings centred on views.rs (plus #8's server.rs half, #34's app.js
half). Cells run STRICTLY SEQUENTIALLY in one worktree — they share views.rs, so
only one worker touches the tree at a time.

## Locked Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D1 | `home_page` never swallows the tab strip when the cross-project bee board is empty: an empty Kanban/cross-project section still renders the tab strip (Kanban/Projects/Terminals), so the Terminals tab stays reachable regardless of bee metadata. | #26; decision d7f85de8. `home_page` currently early-returns `project_list_page` (no tabs) when `cross_features_html.is_empty()`. |
| D2 | An agentless/shell pane renders a non-empty accessible name (fall back to `shell`) so aria-labels read `Scroll shell's history` / `Reply to shell`, never an empty interpolation. | #8; decision 576e7a3c. `project_panes` yields empty name → malformed aria. |
| D3 | When any `.bee` file fails to read (`read_errors` non-empty, folded into `compute_attention_items`), the cross-project board shows ONE concise warning strip — e.g. "N .bee file(s) could not be read; counts may be incomplete" — not the removed Needs-attention panel. | #31; decision 0f2e04fa. Keeps board-trim's no-panel spirit while ending silent invisibility of corrupt `.bee` files. |
| D4 | The two inline `<script>` consts in views.rs (`UNASSIGNED_TERMINAL_SCRIPT`, `TERMINAL_CREATE_SCRIPT`) are folded into `assets/app.js` — one shared script parameterised by the page's own data attributes — so the terminal client logic lives in one place a JS linter can see. | #34; the consts already carry a doc comment deferring exactly this fold. |
| D5 | `bee_hub_card`'s 15 positional params collapse into one `BeeHubCardData`-style struct passed by reference; all ~23 call sites updated. | #32; four adjacent `Option<&str>` currently swap silently. Maintainability only, no behavior change. |

### Agent's Discretion
- D3: exact strip wording/placement in the board header, matching existing board styling.
- D4: how app.js is parameterised (data-attributes already present vs new ones); keep every terminal page behaving exactly as now — this is a refactor, not a behavior change.
- D5: struct name and whether it reuses an existing card-data struct.

## Existing Code Context (HEAD anchors)
- #26 `views.rs:228-230` home_page early return.
- #8 `server.rs:2966` project_panes name; `views.rs:1206,1294,1362` aria sites.
- #31 `compute_attention_items` (waggledance-core/src/bee.rs); board render header in views.rs.
- #34 `views.rs:1388` TERMINAL_CREATE_SCRIPT, `views.rs:1682` UNASSIGNED_TERMINAL_SCRIPT; app.js.
- #32 `views.rs:3723-3739` bee_hub_card + 23 call sites.

## Verify
`cargo test --workspace` green (CI triple: fmt + clippy + test). Each cell adds/keeps
tests: #26 a home_page test that an empty-board home still renders the tab strip and
Terminals tab; #8 an aria test that a shell pane's labels are non-empty; #31 a board
test that a snapshot with read_errors renders the strip and a clean one renders none;
#34 keep all terminal tests green (behavior unchanged); #32 pure refactor, existing
card tests must stay green.

## Handoff
Cells run one at a time in this worktree. CONTEXT is source of truth; D1-D5 stable.
