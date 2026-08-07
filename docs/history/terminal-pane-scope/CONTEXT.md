# terminal-pane-scope — locked context

The owner chose all three of the decisions below directly, in one answer, from
options that named their consequences. Shaping's interview was therefore not
run — a recorded deviation, not an omission: there were no gray areas left to
interview about, and re-asking would have re-litigated a settled choice.

## What prompted this

herdr's own sidebar lists every workspace's agents with a clear status; the
dashboard's Terminal tab lists fewer panes than the owner expects to see.
Measured live against the running herdr socket on 2026-08-07:

- Workspace `bee-dashboard` holds 4 panes. Two of them sit inside this repo:
  `w5:p5` (a claude agent) and `w5:p6` (a plain shell). The project's Terminal
  page rendered exactly one card, `w5:p5`.
- `w5:p5` reports `cwd=/home/thanhsmind/projects/goglbe/beedashboard` but
  `foreground_cwd=/home/thanhsmind/projects/goglbe/beedashboard--wt--agent-terminal`.
  The registered project for that worktree therefore listed **zero** panes
  while a claude session was live inside it.
- Multiple agents per project already render correctly — project
  `anphabe-bi-dashboard` rendered both `w6:p1` and `w6:p5`. Nothing dedupes or
  collapses; the loss is entirely in which panes qualify.

## Decisions (locked)

**D1 — Membership accepts either directory.** A pane belongs to a project when
either its `cwd` or its `foreground_cwd` resolves inside that project's
containment boundary. An agent that works inside a git worktree lists under
that worktree's own project. A pane may legitimately qualify for two projects
at once (a parent repo and its worktree); that is accepted, not resolved.

**D2 — Shell panes are listed.** The project's Terminal and Transcript lists
show every pane inside the boundary, agent or not. A pane herdr reports no
agent for renders as a shell row.

**D3 — Each card states its identity and status.** A card carries its
workspace-and-tab label and a status glyph for working / idle / done /
blocked, the way herdr's sidebar reads. Today's card shows bare status text
with no workspace or tab identity.

**D4 — One pane at a time, one URL each.** The Terminal and Transcript tabs
show a single pane, chosen from a pane tab strip in which every pane carries
its own address. Today both stack every pane onto one shared page: the owner's
`anphabe-bi-dashboard` page rendered two full terminal cards — screen, arrows,
key row, reply box — one under the other, and no link exists that opens just
one of them.

**D5 — The arrow pad is a real touch target.** The four screen-moving arrows
sit in a 44px box with the glyph at body size. The named keys beside them
(Enter, Esc, Tab) stay small. The arrows are pressed repeatedly while reading,
often with a thumb; the named keys are pressed occasionally and deliberately.

## What these decisions endanger

`project_panes` is not only the list — it is the **authorization check** for
`/_terminal/:pane_id/screen`, `/input`, and `/keys`
(`crates/mdview/src/server.rs:929-934`, and the same join in
`terminal_transcript`). The terminal family carries no authentication by an
earlier locked decision (terminal-open-access). Widening membership therefore
widens what an unauthenticated visitor can read from and type into. That is
the intended effect of D1 and D2, and it is the reason this feature routes
high-risk: the containment boundary itself must keep refusing every pane that
resolves outside the project root, on both directories, with proof.

## Out of scope

- The `unassigned` group's own switch and default (untouched).
- Authentication for the terminal family (settled elsewhere).
- The temporary `screen history read` log line at `server.rs:938` — a separate
  open item from an earlier session's handoff.
