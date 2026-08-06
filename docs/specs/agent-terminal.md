---
area: agent-terminal
updated: 2026-08-06
sources: [agent-terminal]
decisions: [D1, D2, D3, D4, D5, D6, D7, D8, D9, D10]
coverage: partial
---

# Spec: Agent terminal

mdview absorbs herdr-go, the standalone mobile-first gateway that watched and
replied to coding agents running under herdr. herdr-go is retired; this is
its successor inside mdview. Every registered project gains a Terminal tab
(watch and reply to the agents running under it) and a Transcript tab (each
agent's own activity log), plus two background duties herdr-go used to
carry, both off until an operator switches them on. mdview never runs a
terminal of its own — it always talks to a running herdr, the same way
herdr-go did.

Technology-agnostic: this describes behavior and rules, not the
implementation. Code entry points are listed in `reading-map.md`.

## Entry Points & Triggers

- A registered project's page → a "Terminal" tab, always present, alongside
  the existing tabs, whether or not the terminal has ever been switched on.
- Opening the Terminal tab → lists every coding agent herdr is running whose
  working directory sits under this project's root, each with its
  live-polled screen and a control to reply, plus a control to start a new
  agent.
- Opening the "Transcript" tab beside it → the same set of agents, each
  showing its own activity log instead of a screen.
- An agent whose working directory sits outside every registered project's
  root → not listed on any project's tab; instead a card on the project list
  page, "Unassigned agents," opens a page listing exactly those agents. The
  card itself carries no agent name and no working directory — it is a bare
  presence marker, visible before anyone has signed in (see Business Rules).
- The settings page → where the terminal's access token is generated and
  rotated, and where the two background duties are switched on, alongside
  every other, unrelated mdview setting. Reaching the page itself never
  requires signing in — see Actors & Access for exactly which parts of it do.

## Data Dictionary

| # | Element | Meaning | Values |
|---|---|---|---|
| 1 | Agent | One coding agent herdr is running, addressed by its own id | id, working directory (via its pane), status, current screen |
| 2 | Pane | The addressable session an agent runs inside; every listed agent has exactly one, but a pane can exist with no agent record attached (a plain shell) — see Open Gaps for what that means today | id, working directory |
| 3 | Screen | The agent's current visible terminal contents, rendered with colour | a snapshot redrawn on each poll, not a live feed |
| 4 | Transcript | The agent's own activity log, read directly rather than through herdr | a gap-free running record of the agent's activity, independent of the screen poll; a fresh agent with nothing written yet reports that plainly rather than showing an empty log |
| 5 | Access token | The one credential that unlocks the terminal, transcript, and agent-creation family | generated/rotated on the settings page; shown in full exactly once, at the moment it is generated or rotated — every later view of the settings page shows only its last few characters |
| 6 | Unassigned agents | Agents whose working directory is outside every registered project's root | listed on their own page, gated the same as any project's agents |
| 7 | Keep herdr running | Opt-in duty: mdview keeps the herdr process alive on the operator's behalf | on / off, off by default |
| 8 | Notify on status change | Opt-in duty: mdview sends a notification message when a watched agent's status changes | on / off, off by default; needs a destination and a credential configured separately |
| 9 | Notify credential | The secret used to send that notification message | write-only: once saved, it is never shown again in full — only a masked hint — and it never appears in any viewed or exported configuration |

## Behaviors & Operations

### Viewing the terminal

- **Triggers:** opening a registered project's Terminal tab with a valid
  session.
- **What it shows:** every agent whose working directory sits under this
  project's root, each with its screen rendered as coloured text and a
  control to reply. An agent belonging to a different project, or to none,
  never appears here.
- **Afterwards:** the operator sees exactly the agents that belong to this
  project.

### Replying to an agent

- **Triggers:** typing free text, sending a named key (for example Enter, an
  arrow key, Ctrl+C), or reading the current screen or the transcript, from
  a listed agent's pane, with a valid session.
- **What it does:** typed text can be staged into the agent's pane without
  being sent — submitting it (pressing Enter) is a separate act the operator
  chooses explicitly; a named key is sent immediately. Every one of these
  actions — reading the screen, sending text, sending a key, reading the
  transcript — is refused unless the target agent already belongs to this
  project, exactly like viewing the terminal itself.
- **Afterwards:** the agent's next screen poll reflects whatever it did with
  the input.

### Starting a new agent

- **Triggers:** using the Terminal tab's creation control — either picking
  one of the presets an operator configured in advance, or opening a plain
  shell.
- **What it does:** for a preset, the request names only the preset's label;
  the command that actually runs is entirely whatever an operator configured
  for that label in advance — a creation request can never influence what is
  run, and it carries no destination either way. For a plain shell, the
  request supplies nothing at all. In both cases the destination is chosen
  automatically: the first working directory herdr already reports as
  belonging to this project, validated against the project's own boundary.
  A preset-started agent appears in the Terminal tab's listing on its next
  poll; a plain shell does not, because that listing enumerates agents, and
  a plain shell has no agent record (see Open Gaps).
- **Blocked when:** no such destination can be found under this project's
  boundary, the named preset is not one an operator configured, or the
  underlying start attempt itself fails — each of these is refused
  distinctly, with nothing started in any case.

### Viewing the transcript

- **Triggers:** opening a registered project's Transcript tab with a valid
  session.
- **What it shows:** the same set of agents as the Terminal tab, each showing
  its own session log instead of a screen. An agent that has not written
  anything yet reports plainly that no transcript is available yet, rather
  than showing an empty frame that could be mistaken for "caught up."
- **Afterwards:** the operator can see what an agent did even for output that
  has since scrolled off or been cleared — something the polled screen alone
  would lose. The screen and the transcript answer different questions and
  are kept as separate tabs rather than merged into one.

### Unlocking the terminal (the token boundary)

- **Triggers:** presenting the access token on the settings page.
- **What it does:** presenting the correct token starts a session; every
  terminal, transcript, and agent-creation request is refused unless it
  carries a valid session for the token currently in effect. Rotating the
  token requires a session under the token being replaced — except the very
  first time, before any token has ever been generated, when rotation is
  open, since there is nothing yet to prove possession of.
- **What's gated:** the Terminal tab and its screen, sending text and keys,
  the Transcript tab, starting a new agent, and the Unassigned agents page
  and its contents — every action listed above, not only viewing and
  creating.
- **What's not gated (unchanged by this feature):** every other page in
  mdview — the project list, a project's markdown pages, search, the
  settings page itself, and the plain status/configuration views all remain
  reachable to anyone who can reach the server, exactly as before this
  feature. The project list in particular reveals no agent name and no
  working directory to a visitor who has never signed in; opening it, or
  opening the Unassigned agents page's presence marker, never adds anything
  to the registered project list either. A request that lacks a valid
  session for a gated action is answered exactly as an address that was
  never used at all would be — and attempting the wrong kind of request
  against a gated address (for example, fetching as a page a link that only
  accepts a submission) gets that identical answer too; nothing distinguishes
  a wrong or missing token, a wrong kind of request, and a page that was
  never there.
- **Afterwards:** rotating the token immediately ends every session that was
  signed in under the previous one.

### When herdr is not running

- **Triggers:** opening the Terminal or Transcript tab, or polling a pane's
  screen, while herdr cannot be reached.
- **What it shows:** an explicit "herdr is not running" message naming the
  remedy — start herdr, then reload the page — instead of an empty or
  broken-looking tab. mdview never starts herdr on its own unless the "keep
  herdr running" duty below is switched on.

### The two background duties

- **Keep herdr running:** when switched on, mdview keeps the herdr process
  alive on the operator's behalf. Off by default; mdview spawns no process of
  its own until this is turned on.
- **Notify on status change:** when switched on, mdview sends a notification
  message when a watched agent's status changes (this also needs a
  destination and a credential configured separately; the credential is
  never shown again in full once saved, and never appears in any viewed or
  exported configuration). Off by default; mdview makes no outbound call
  until this is turned on.
- Both duties, together with the notification destination and credential,
  are switched on and changed from the settings page — but doing so requires
  a live terminal session, unlike every other setting on that same page (see
  Actors & Access). They take effect immediately without a restart.

## Actors & Access

One local operator per install, same as the rest of mdview. Unlike every
other surface in mdview, the terminal, transcript, and agent-creation family
is gated: reaching any of it requires the session that presenting the access
token establishes (see the token boundary above).

The settings page that hosts the token is itself unauthenticated, like the
rest of mdview outside this family — reaching it is enough to view or change
every ordinary setting (server binding, theme, indexing, and so on), and,
once a token exists, to attempt signing in with it. But three things reached
from that same page are carved out and require an existing session before
they take effect: turning "keep herdr running" or "notify on status change"
on or off, and changing the notification destination or credential.
Reaching those through the ordinary, unauthenticated settings path would let
any visitor on the network make mdview start a process or begin sending
notifications on the operator's behalf — the one danger D4's gate exists to
close. Everything else on the settings page stays as open as it always was.

## Business Rules

- **Project scoping (D2).** An agent's working directory decides which
  project, if any, lists it; an agent under no registered project's root
  appears only in the Unassigned group, never silently on some other
  project's tab. This boundary is enforced on every action that names a
  pane — viewing its screen, sending text, sending keys, reading its
  transcript, listing it, and choosing where a new agent starts — never only
  on listing and creation. It refuses a working directory that escapes the
  project's root either by walking up through parent directories or by
  following a symbolic link out of it.
- **Nothing lost (D5).** An agent whose working directory is outside every
  registered project is never dropped from view — it always appears, in the
  Unassigned group, behind the same gate as everything else. The registered
  project list is never changed by any of this: listing, or even opening, an
  agent never adds its project to the registry.
- **Tab always present (D6).** The Terminal tab (and Transcript tab) render
  on every registered project's page whether or not the terminal has ever
  been switched on or herdr is reachable; a missing herdr is a named state,
  never a hidden tab.
- **Off until switched on (D7).** The terminal family, the "keep herdr
  running" duty, and the "notify on status change" duty are each
  independently off until an operator turns them on from the settings page;
  none of them changes mdview's behavior for an install that never visits
  that page.
- **Screen vs. transcript (D9).** The screen is a periodic, coloured snapshot
  redrawn on each poll; the transcript is the agent's own gap-free log. They
  are kept as two tabs rather than one, because collapsing them loses
  whichever one isn't currently showing.
- **An agent is not its pane.** "Agent" is what the operator sees and picks —
  a coding agent herdr is running. "Pane" is the session it runs inside.
  Every agent has exactly one pane, but the reverse does not hold: a plain
  shell opened with no agent started in it is a pane with no agent record,
  and today it is invisible to the operator once created — not listed on any
  project's tab, not in the Unassigned group, addressable by nothing the
  Terminal tab exposes. Whether that gap should be closed by showing
  agentless panes too, or by treating "Unassigned"/D5 as being about panes
  rather than agents throughout, is open — see Open Gaps.

## Edge Cases Settled

- Terminal switched off: the tab answers exactly as an address that was
  never used would, even for a visitor who already holds a valid session.
- Herdr not reachable: every affected view degrades to the named "herdr is
  not running" state rather than an error page or a blank screen.
- An agent whose working directory is outside every registered project: it
  is never dropped — it appears in the Unassigned group instead.
- An agent with no transcript written yet: answered as a named, successful
  "nothing yet" state, never as an empty list indistinguishable from "caught
  up, nothing new."

## Open Gaps

- **A plain shell is created but not addressable afterward.** D5's wording
  says "panes"; herdr keeps agents and panes as independent lists, and both
  the project listing and the Unassigned group iterate agents only. A plain
  shell started through the creation control therefore never becomes visible
  or reachable anywhere in the terminal surface once it exists. This is the
  user's call, not something this spec resolves — recorded as an outstanding
  question in `docs/history/agent-terminal/CONTEXT.md`.
- Confirmation against a real, running herdr (rather than a test double) is
  a manual check at UAT, not something automated coverage certifies.
- Mobile-specific layout for this surface, carried over as an idea from
  herdr-go's mobile-first design, is not settled.

## Visuals

No settled screenshot captured yet.

## Pointers (implementation)

- `crates/mdview/src/server.rs` — the routes themselves (`/p/:id/_terminal`,
  `/p/:id/_transcript`, their screen/input/keys/create children, and the
  `/_terminal/unassigned` family), each gated behind `AuthSession` and the
  terminal-enabled switch before it does anything else; `project_panes`
  (agents joined to their pane's cwd, filtered by the D2 boundary) is what
  makes a plain shell invisible to listing and to every pane-scoped action
  (`project_and_verify_pane_in_boundary`, `project_pane_cwd_in_boundary`);
  `CreatePaneBody`/`CreateAgentBody` are the (empty / preset-label-only)
  request shapes a creation call actually sends; `update_terminal_config` is
  the one route that gates the D7 switches plus the notify destination/credential
  behind `AuthSession`, distinct from the unauthenticated `/api/config`.
- `crates/mdview/src/terminal_auth.rs` — the token/session mechanism: token
  storage, reveal-once rotation (`rotate_terminal_token`, gated once
  `is_configured()`), and `MethodGate`/`opaque_404`, the shared answer a
  failed session or a mismatched request kind gets.
- `crates/mdview/src/herdr/` — the client that talks to a running herdr over
  its socket (or named pipe on Windows).
- `crates/mdview/src/supervisor.rs`, `crates/mdview/src/notify/` — the two
  opt-in background duties.
- `crates/mdview/src/views.rs` — the tab pages, screen/transcript rendering,
  the herdr-down state's wording, and `project_list_page`'s presence-only
  Unassigned card.
- `crates/mdview-core/src/config.rs` — the D7 switches and the agent-create
  presets in `Config`; `masked_notify_credential`/`save_notify_credential`
  keep the notify credential write-only.
- `crates/mdview-core/src/transcript.rs` — reading an agent's own session
  log.
- `crates/mdview-core/src/paths_boundary.rs` — the containment check that
  scopes panes to a project's root (`Boundary::validate_existing`'s 7 steps,
  including traversal rejection and symlink resolution).
- `crates/mdview-core/src/notify_store.rs` — the notification outbox used
  only when the notify duty is on.
- `crates/mdview-core/src/ansi.rs` — translating a raw screen into safe,
  coloured HTML.
- `docs/history/agent-terminal/CONTEXT.md` — Outstanding Questions, for the
  agent-vs-pane gap above.
