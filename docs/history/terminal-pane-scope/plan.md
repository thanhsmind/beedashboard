# terminal-pane-scope — implementation plan

Lane: **high-risk** — flags `audit-security`, `covered-contract-change`,
`proof-weakening`. Product files: 3 (`crates/mdview/src/server.rs`,
`crates/mdview/src/views.rs`, `crates/mdview/src/herdr/fake.rs`).
Decisions: `docs/history/terminal-pane-scope/CONTEXT.md` (D1–D5).

Revision 2 — the owner added D4 (one pane per tab, each with its own URL) and
D5 (a real arrow pad) after reading revision 1, which covered D1–D3 only. The
shape gate was unapproved and redrafted rather than stamped, because D4 changes
what a page *is*, not how it looks.

Revised after a review pass against the code; every correction it found is
folded in below and named in "What review changed".

## Why this is high-risk and not a display tweak

`project_panes` (`server.rs:1448`) is the single membership function, and six
call sites ask it for permission, not for a list:

| site | route | what the widening does there |
|---|---|---|
| `server.rs:710` | `terminal_page` | lists more panes — the intended effect |
| `server.rs:739` | `transcript_page` | lists more panes; each new card starts polling the transcript route |
| `server.rs:930` | `terminal_screen` | **authorization widens** — screen read |
| `server.rs:1017` | `project_and_verify_pane_in_boundary` → `terminal_input` (`:1181`), `terminal_keys` (`:1237`) | **authorization widens** — keystroke injection |
| `server.rs:1047` | `project_pane_cwd_in_boundary` → `terminal_transcript` (`:1108`) | authorization widens, **and the value it returns keys the transcript read** |
| `server.rs:1513` | inside `unassigned_panes` | `assigned` grows, so the Unassigned group **shrinks** — the fail-safe direction |

The terminal family has no authentication (locked earlier, terminal-open-access,
D4 there). So every pane this function starts accepting becomes a pane an
unauthenticated visitor on the LAN can read and send keystrokes to.

**The surface the correct implementation opens, stated plainly.**
`foreground_cwd` is not a property of the pane — it is the directory of
whatever process is in the foreground *at poll time* (`wire.rs:139-142`).
Together with D2's shell rows, membership becomes: *any pane on this host,
for as long as its foreground process is inside a registered project root.*
Someone who types `cd ~/projects/goglbe/beedashboard` in an unrelated terminal
hands that pane to the project's page until they `cd` away. This is the
decisions' intent, not a defect — but it is the fact the gate is deciding on,
and it is why every case below exists.

The create routes are the one terminal-family path that does **not** ask
`project_panes`: they authorize a workspace anchor through
`project_creation_destination` (`server.rs:1272-1283` → `wire.rs:296`), which
already prefers `foreground_cwd ?? cwd`. They are unaffected in both
directions, and `terminal_create_routes_refuse_a_destination_outside_the_project_root`
(`server.rs:10005`) therefore proves nothing about this change.

## The shape

One function changes meaning; everything else follows from it.

Today `project_panes` iterates `snapshot.agents` and joins each to its pane for
a `cwd`. Both halves are wrong for the decisions: agents are a subset of panes
(D2), and `cwd` is one of two directories a pane reports (D1).

Inverted, it iterates `snapshot.panes` — the set membership is actually about —
accepts a pane when `cwd` **or** `foreground_cwd` validates inside the
boundary, and joins the *optional* agent by `pane_id`. A pane with no agent is
a shell row rather than an absence.

This is not a reversal of the area's own decision, though it does reverse the
rationale written at `server.rs:1438-1447` ("`foreground_cwd` is not consulted
here"). agent-terminal's D2 (`docs/history/agent-terminal/CONTEXT.md:28`) reads
"lists only the herdr **panes** whose working directory sits under that
project's `root_path`" — pane iteration was the decision's own wording, and
the code has been narrower than its decision since it was written. That doc
comment gets rewritten with the new rationale, not silently contradicted.

### Two rules the review found unspecified, now settled

**Precedence: `cwd` wins when both validate.** The path `project_panes` returns
is not display-only — `project_pane_cwd_in_boundary` (`server.rs:1034-1054`)
hands it to `read_activity`, and `transcript.rs:122-123` uses it as the
transcript *directory selector*. Preferring `foreground_cwd` would silently
re-key every existing pane's transcript from its launch directory to its live
one. So: try `cwd` first; fall back to `foreground_cwd` only when `cwd` is
absent or refused.

A consequence, accepted: on the *worktree* project's page, the motivating pane
matches only via `foreground_cwd`, so its transcript is keyed on the worktree
path — where Claude Code, which writes its transcript dir from the session's
launch cwd, has nothing. That tab answers `available: false`, which is the
honest empty state, not an error. Case 8 pins it.

**`unassigned_panes` keeps its agent-only output loop** (`server.rs:1525-1528`).
Only the `assigned` set it subtracts (`server.rs:1513`) grows with the
inversion, so the group can only shrink — the fail-safe direction. Inverting
its output loop too would newly expose *every shell pane on the host* — root
shells, unrelated repos — to unauthenticated read and keystroke injection
through `unassigned_terminal_screen` / `_input` / `_keys` (`server.rs:1620`,
`:1662`, `:1687`), a group whose own doc says it has no containment check of
its own (`server.rs:792-799`). CONTEXT.md puts that group out of scope, and
this plan holds that line.

Known gap this leaves standing, unchanged and pre-existing: a shell pane under
no registered project appears nowhere. agent-terminal's D5 says panes should
surface in the Unassigned group; today only agents do. That gap is older than
this feature and is not widened by it. Case 6 pins the behavior so a later
feature changes it deliberately.

**Windows: D1 is a no-op there.** `foreground_cwd` is unix-only, `None`
elsewhere (`wire.rs:139`). The `foreground_cwd` cases carry `#[cfg(unix)]`, the
way `server.rs:6869` already does.

### Slice 1 — the two cells

**Cell 1 · Membership: panes, not agents; either directory, `cwd` first.**
`project_panes` iterates `snapshot.panes`; a pane qualifies when
`boundary.validate_existing` accepts `cwd`, else `foreground_cwd`; the resolved
path of whichever matched is what the row carries. The agent is joined
optionally: present → today's `kind`/`name`/`status`/`title`; absent → a shell
row. `unassigned_panes`'s output loop is left alone, and the `server.rs:1438-1447`
rationale is rewritten. `fake.rs` gains what these tests need — a pane whose
`foreground_cwd` differs from its `cwd`, on paths that really exist so
`canonicalize` accepts them (`paths_boundary.rs:145-146`); today's `pane()`
helper hard-wires `foreground_cwd == cwd` (`fake.rs:522-531`) and every other
fixture does the same (`fake.rs:489`, `fake.rs:729`, `server.rs:9821`).

**Cell 2 · The card states its identity and status.**
`TerminalPaneView` carries the workspace label, the tab label, and a status
that admits having no agent. The join is already written and currently unused
by any non-test caller: `Snapshot::workspace_label_for_id` (`wire.rs:221`) and
`tab_label_for_id` (`wire.rs:242`), whose own doc comments name the shell-row
case. `pane_cards` and `transcript_cards` (`views.rs:298`, `views.rs:487`)
render `<workspace> · <tab>` as the card's identity and replace the flat
`fg-chip fg-chip--neutral` (`views.rs:306`, `views.rs:495`) with the design
system's existing `.fg-status` / `.fg-status__dot` pill
(`crates/mdview/assets/atelier/components.css:145-151`, already compiled into
every page via `views.rs:2947`). Six states map onto its three modifiers with
no CSS edit: `done` → `--ready`, `working` → `--warn`, `blocked` → `--blocked`,
and `idle` / `unknown` (`wire.rs:27-28`) / shell → the unmodified neutral dot,
each still named in the pill's own text. A shell row reads as a shell instead
of borrowing an agent's vocabulary.

**Cell 3 · One pane per tab, one URL each (D4).**
`terminal_page` and `transcript_page` stop rendering every pane and render
one. A pane tab strip sits under the existing Overview / Terminal / Transcript
nav (`views.rs:258`), one entry per pane in the project's list, each entry an
ordinary link carrying that pane's own address:
`/p/:id/_terminal/pane/:pane_id` and `/p/:id/_transcript/pane/:pane_id`. The
`pane/` segment is explicit so no pane id can ever shadow the existing
`/_terminal/create/pane` route. The bare `/_terminal` and `/_transcript` keep
working and select a default — the snapshot's focused pane when it is one of
this project's, else the first — so every link that exists today still opens
something. A pane id that is not in this project's list gets the ordinary
not-found page, the same refusal `terminal_screen` already makes
(`server.rs:930`); a project with no panes keeps today's honest empty state.
Each strip entry carries the identity and status pill cell 2 builds, so the
strip reads the way herdr's own sidebar does. No JavaScript: these are links,
and one pane per page means one screen poller instead of N.

**Cell 4 · Make the arrow pad hittable (D5).**
In `views.rs`'s `PROJECT_TAB_STYLE`, the four screen-moving arrows — the
`.term-keys` that sits directly under `.term-controls`, not the named-key row
inside `.term-controls__row` — get a 44px minimum box with the glyph at
`--type-body-size`. Existing tokens only; the named keys, the scroll pair and
the reply buttons are untouched, and the handset media query at `views.rs:214`
keeps its own padding bump.

Slice 1 is the whole feature; there is no slice 2.

### Smaller path check

*Is there a cheaper shape that still honors D1, D2, D3?* **No.** The one
cheaper candidate — keep iterating `agents` and add a second `agents`-side
lookup for `foreground_cwd` — satisfies D1 but structurally cannot satisfy D2:
a shell pane never appears in `agents[]` at all (`wire.rs:126-130` says so
outright, and `fake.rs:174`/`fake.rs:185` are fixture proof). Inverting the
iteration is the minimum that reaches both.

## Proof

`commands.test` is `cargo test --workspace` (`.bee/config.json:13`), run at
every cap.

Existing tests that already fence this area and must stay green unchanged —
they are what proves the widening did not become a leak for *agent* panes:

- `terminal_route_lists_only_panes_within_the_project_root_boundary`
  (`server.rs:6871`) — one dir above the root, a symlink escaping the root, and
  another project's pane all stay excluded.
- `terminal_screen_refuses_a_pane_outside_the_project_root` (`server.rs:7143`),
  `terminal_transcript_refuses_a_pane_outside_the_project_root`
  (`server.rs:7913`), `terminal_write_routes_refuse_a_pane_outside_the_project_root`
  (`server.rs:8625`) — the authorization edge, per route.
- `unassigned_group_fails_closed_when_a_projects_boundary_is_unconstructable`
  (`server.rs:9543`) — the group's fail-closed rule.
- `unassigned_group_and_a_projects_own_terminal_partition_panes_without_overlap`
  (`server.rs:9032`) — the partition. Note it asserts on *agent names*
  (`server.rs:9069-9088`), so it cannot see shell rows; case 6 is what covers
  them.

New cases the cells own — the gap, not a re-assertion of the above. Every
listing case asserts on **pane id**, never on an agent name, so a shell row is
visible to the assertion:

1. A pane inside the root with **no** agent is listed, as a shell row (D2).
2. `#[cfg(unix)]` A pane whose `cwd` is outside the root but whose
   `foreground_cwd` is inside is listed, and its screen route answers (D1) —
   the worktree case measured in CONTEXT.md.
3. A pane whose `cwd` is inside the root but whose `foreground_cwd` is outside
   is listed (D1, the mirror direction).
4. A pane where **neither** directory is inside the root stays excluded, and
   its screen, input, and keys routes all still 404 — the widening's outer edge.
5. `#[cfg(unix)]` A pane whose `foreground_cwd` escapes via a symlink to
   outside the root is refused, matching the `cwd` symlink case already proven
   at `server.rs:6871`.
6. A shell pane under **no** registered project is absent from the project's
   list **and** absent from the Unassigned group — the standing gap, pinned by
   pane id so a later feature has to change it on purpose.
7. A pane reporting **neither** `cwd` nor `foreground_cwd` is excluded from the
   project's list.
8. `#[cfg(unix)]` For a pane matched only via `foreground_cwd`, the transcript
   route keys on that matched path — answering `available: false` when nothing
   is written there — and a pane whose `cwd` validates keys on `cwd` even when
   `foreground_cwd` also validates, proving the precedence rule.
9. The same pane qualifying for two registered projects is listed by both, and
   each project's screen route serves it under its own boundary.
10. A card renders its workspace and tab label, and a status pill whose class
    differs between `working`, `idle`, `done`, and `blocked` (D3).
11. A shell row renders without an agent kind, and does not claim a status it
    does not have (D2 + D3).
12. A project with two panes renders a strip with two entries carrying two
    different hrefs, and each page renders exactly one screen — not both (D4).
13. `/_terminal/pane/:pane_id` for a pane outside the project answers the
    not-found page and never names that pane, matching `server.rs:7143`'s
    refusal for the screen route (D4, the same authorization edge one level up).
14. The bare `/_terminal` still answers, selecting the focused pane when it is
    in the project and the first otherwise; a project with no panes keeps its
    honest empty state rather than 404ing (D4).
15. The four arrows carry a larger minimum box than the named keys beside
    them, which are unchanged (D5).

Cases 4 and 5 are the ones that would turn this feature into a vulnerability if
missed: `validate_existing` is what resolves symlinks, so applying it to the
second directory is not optional.

## Cost if the shape is wrong

Wrong in the permissive direction, an unauthenticated visitor can type into a
pane outside the project root — the failure class the containment boundary
exists to prevent. Wrong in the restrictive direction, the page lists nothing
and the regression is loud and harmless. Cases 4, 5, and 6 are ordered so the
permissive failure cannot pass silently.

Separately from implementation error, the *correct* implementation still widens
real exposure exactly as much as the section above describes. If that trade is
not wanted, the lever is D1 — not this plan.

## What review changed

- The plan said `unassigned_panes` both stays unchanged and mirrors the new
  primitive. Contradiction resolved: its output loop stays agent-only, and the
  reason is now stated (case 6).
- Precedence between `cwd` and `foreground_cwd` was unspecified while that
  value keys the transcript read. Now `cwd`-first, with case 8.
- The exposure of the *correct* implementation was never stated — only the cost
  of getting it wrong. Now stated up front.
- Cases added: neither directory present (7), same pane in two projects (9),
  the standing Unassigned gap (6), transcript precedence (8).
- `AgentStatus::Unknown` is a fifth status and shell a sixth; the mapping onto
  `.fg-status`'s three modifiers is now explicit, and no CSS file is touched.
- Windows: D1 is a no-op there; `foreground_cwd` cases are `#[cfg(unix)]`.
- Corrected citations: `wire.rs:111`/`123` are the `label` fields (107/120 were
  their doc comments); the "never appears in `agents[]`" sentence runs
  `wire.rs:126-130`; the stylesheet is `crates/mdview/assets/atelier/components.css`.
- Dropped the claim that the create-route test fences this change — it doesn't.
- Cell 2's work is smaller than claimed: the label helpers already exist.
