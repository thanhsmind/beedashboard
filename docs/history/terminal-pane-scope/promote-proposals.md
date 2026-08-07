promote proposal for work item "terminal-pane-scope" (.bee/logs/scribing-runs.jsonl + .bee/lanes/terminal-pane-scope.json) — 4 capped cell(s): terminal-pane-scope-1, terminal-pane-scope-2, terminal-pane-scope-3, terminal-pane-scope-4
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/terminal-pane-scope.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/terminal-pane-scope/delivery.md

---
type: bee.delivery
title: terminal-pane-scope — delivery
description: "Delivery record proposed by bee knowledge promote for work item terminal-pane-scope: 4 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-07
bee:
  id: terminal-pane-scope-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/terminal-pane-scope.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/terminal-pane-scope.json, .bee/cells/terminal-pane-scope-1.json, .bee/cells/terminal-pane-scope-2.json, .bee/cells/terminal-pane-scope-3.json, .bee/cells/terminal-pane-scope-4.json]
---

# terminal-pane-scope — Delivery

## What shipped

- **terminal-pane-scope-1** — project_panes now iterates snapshot.panes with cwd-first/foreground_cwd-fallback membership and an optional agent join; shell rows render, unassigned stays agent-only, 9 new boundary tests added (2 file(s) changed)
- **terminal-pane-scope-2** — Cards render <workspace> · <tab> identity and an fg-status pill (done/working/blocked mapped, idle/unknown/shell neutral); shell rows now claim status 'shell' instead of blank (2 file(s) changed)
- **terminal-pane-scope-3** — Terminal and Transcript pages render one pane each, chosen via a pane tab strip with per-pane URLs (2 file(s) changed)
- **terminal-pane-scope-4** — Scoped the 44px arrow target to .term-controls > .term-keys button, leaving named keys/scroll/reply untouched; added a render test (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **terminal-pane-scope-1** — `cargo test --workspace, with every existing boundary test still green and unchanged: terminal_route_lists_only_panes_within_the_project_root_boundary (server.rs:6871), terminal_screen_refuses_a_pane_outside_the_project_root (:7143), terminal_transcript_refuses_a_pane_outside_the_project_root (:7913), terminal_write_routes_refuse_a_pane_outside_the_project_root (:8625), unassigned_group_and_a_projects_own_terminal_partition_panes_without_overlap (:9032), unassigned_group_fails_closed_when_a_projects_boundary_is_unconstructable (:9543). New route-level tests through tower ServiceExt oneshot, every listing assertion made on pane id and never on an agent name so a shell row is visible to it: (happy) a pane inside the root with no agent is listed as a shell row; (happy, cfg unix) a pane whose cwd is outside the root but whose foreground_cwd is inside is listed and its screen route answers; (happy) a pane whose cwd is inside but whose foreground_cwd is outside is listed; (security) a pane with neither directory inside the root is absent from the list and its screen, input and keys routes all 404; (security, cfg unix) a pane whose foreground_cwd escapes by symlink to outside the root is refused, mirroring the cwd symlink case already at :6871; (edge) a pane reporting neither cwd nor foreground_cwd is excluded; (regression) a shell pane under no registered project is absent from the project list AND absent from the unassigned group, pinning the standing gap by pane id; (edge, cfg unix) a pane matched only through foreground_cwd keys its transcript on that matched path and answers available:false when nothing is written there, while a pane whose cwd validates keys on cwd even when foreground_cwd also validates; (edge) the same pane qualifying for two registered projects is listed by both and each project's screen route serves it under its own boundary.`
- **terminal-pane-scope-2** — `cargo test --workspace, with terminal_page_renders_the_reply_bar_and_key_buttons (server.rs:8743) and transcript_page_renders_the_tab_and_a_viewport_per_pane (:8150) still green. New route-level tests through tower ServiceExt oneshot: (happy) a card renders its workspace label and its tab label together as the card's identity, on both the terminal and the transcript tab; (happy) a working, an idle, a done and a blocked pane each render a status pill whose class differs from the others; (edge) an agent whose status is unknown renders the neutral pill rather than borrowing another state's colour; (happy) a shell row renders no agent kind and names itself a shell; (regression) the reply form, the key buttons and the scroll buttons are still present on every terminal card including a shell row, and the transcript card still carries no screen element.`
- **terminal-pane-scope-3** — `cargo test --workspace, with terminal_page_renders_the_reply_bar_and_key_buttons (server.rs:8743), transcript_page_renders_the_tab_and_a_viewport_per_pane (:8150), terminal_family_disabled_answers_with_the_disabled_shapes (:6966) and transcript_family_disabled_answers_with_the_disabled_shapes (:7863) all still green - updated only where they assert one-page-holds-every-pane, which is the behavior this cell deliberately changes. New route-level tests through tower ServiceExt oneshot: (happy) a project with two panes renders a strip with two entries carrying two different hrefs, and each pane page renders exactly one screen element, not two; (security) /_terminal/pane/:pane_id for a pane outside the project answers the not-found page and the body never contains that pane's id or cwd, mirroring :7143; (edge) the bare /_terminal selects the focused pane when it belongs to the project and the first pane when it does not; (edge) a project with no panes answers the honest empty state, not a 404; (regression) the disabled switch still turns both new routes into the family's own disabled shapes.`
- **terminal-pane-scope-4** — `cargo test --workspace, with the existing render tests still green - terminal_page_renders_the_reply_bar_and_key_buttons (server.rs:8743) in particular, since it asserts the key buttons' presence and DOM order. New test: (happy) the served terminal page carries a rule giving the arrow group's buttons a larger minimum box than the named-key row beside them, and the named keys, the scroll pair and the reply buttons carry no such rule.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work terminal-pane-scope` from 4 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/terminal-pane-scope.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "terminal-pane-scope" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-07T10:13:58.859Z), the work item declares no bee.areas.

area agent-terminal:
  - [terminal-pane-scope-1] project_panes now iterates snapshot.panes with cwd-first/foreground_cwd-fallback membership and an optional agent join; shell rows render, unassigned stays agent-only, 9 new boundary tests added — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/terminal-pane-scope-1.json)
  - [terminal-pane-scope-2] Cards render <workspace> · <tab> identity and an fg-status pill (done/working/blocked mapped, idle/unknown/shell neutral); shell rows now claim status 'shell' instead of blank — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/terminal-pane-scope-2.json)
  - [terminal-pane-scope-3] Terminal and Transcript pages render one pane each, chosen via a pane tab strip with per-pane URLs — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/terminal-pane-scope-3.json)
  - [terminal-pane-scope-4] Scoped the 44px arrow target to .term-controls > .term-keys button, leaving named keys/scroll/reply untouched; added a render test — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/terminal-pane-scope-4.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 4 capped cell(s) mined, 1 delivery draft, 4 area bullet(s), 0 pattern candidate(s), 0 file(s) written.