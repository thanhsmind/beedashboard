---
area: agent-terminal
updated: 2026-08-07
sources: [agent-terminal, terminal-open-access]
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
  presence marker, shown only while both the terminal switch and the
  Unassigned group's own switch are on (see Business Rules); with either
  off, the card does not appear at all.
- The settings page → where the terminal switch, the Unassigned group's own
  switch, and the two background duties are turned on, alongside every
  other, unrelated mdview setting — see Actors & Access for what this
  surface's safety now rests on.

## Data Dictionary

| # | Element | Meaning | Values |
|---|---|---|---|
| 1 | Agent | One coding agent herdr is running, addressed by its own id | id, working directory (via its pane), status, current screen |
| 2 | Pane | The addressable session an agent runs inside; every listed agent has exactly one, but a pane can exist with no agent record attached (a plain shell) — see Open Gaps for what that means today | id, working directory |
| 3 | Screen | The agent's recent terminal contents, rendered with colour | a snapshot redrawn on each poll, not a live feed; a bounded tail of the pane's own scrollback rather than only the rows currently on screen, so a plain shell shows work that has already scrolled past, while an agent that redraws a full-screen interface has no scrollback to give and shows exactly its current frame; shown at the full height of one pane frame with its lines unwrapped, the box scrolling in both directions rather than re-flowing the frame |
| 4 | Transcript | The agent's own activity log, read directly rather than through herdr | a gap-free running record of the agent's activity, independent of the screen poll; a fresh agent with nothing written yet reports that plainly rather than showing an empty log; if the record is found truncated or rewritten under the reader, the next read shows a visible divider rather than jumping silently; a single poll returns only a bounded number of lines, and when a poll has more than that bound, its oldest lines are marked as lost rather than silently dropped |
| 5 | Terminal switch | The one switch standing between anyone who can reach the daemon and the terminal, transcript, and agent-creation family — there is no credential behind it | on / off, off by default |
| 6 | Unassigned agents | Agents whose working directory is outside every registered project's root | listed on their own page, reachable only while both the terminal switch and this group's own switch (below) are on |
| 7 | Unassigned group switch | The Unassigned group's own switch, separate from the terminal switch above — turning the terminal switch on alone does not open this group | on / off, off by default |
| 8 | Keep herdr running | Opt-in duty: mdview keeps the herdr process alive on the operator's behalf | on / off, off by default |
| 9 | Notify on status change | Opt-in duty: mdview sends a notification message when a watched agent's status changes | on / off, off by default; needs a destination and a credential configured separately |
| 10 | Notify credential | The secret used to send that notification message | write-only: once saved, it is never shown again in full — only a masked hint — and it never appears in any viewed or exported configuration; if a save fails, the operator is told it failed — it is never reported as saved |

## Behaviors & Operations

### Viewing the terminal

- **Triggers:** opening a registered project's Terminal tab while the
  terminal switch is on.
- **What it shows:** every agent whose working directory sits under this
  project's root, each with its screen rendered as coloured text and a
  control to reply. An agent belonging to a different project, or to none,
  never appears here.
- **Afterwards:** the operator sees exactly the agents that belong to this
  project.

### Replying to an agent

- **Triggers:** typing free text, sending a named key (for example Enter, an
  arrow key, Ctrl+C), or reading the current screen or the transcript, from
  a listed agent's pane, while the terminal switch is on.
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

- **Triggers:** opening a registered project's Transcript tab while the
  terminal switch is on.
- **What it shows:** the same set of agents as the Terminal tab, each showing
  its own session log instead of a screen. An agent that has not written
  anything yet reports plainly that no transcript is available yet, rather
  than showing an empty frame that could be mistaken for "caught up."
- **Afterwards:** the operator can see what an agent did even for output that
  has since scrolled off or been cleared — something the polled screen alone
  would lose. The screen and the transcript answer different questions and
  are kept as separate tabs rather than merged into one.

### The terminal switch

The terminal has **no authentication of its own** — no token, no session, no
login, no cookie. The switch below is the only thing standing between it and
anyone who can reach the daemon.

- **Triggers:** the "Enable the terminal" switch on the settings page, on or
  off.
- **What it does:** on, every terminal, transcript, and agent-creation route
  answers normally to anyone who can reach the daemon. Off, every one of
  those routes is refused.
- **What's gated:** the Terminal tab and its screen, sending text and keys,
  the Transcript tab, and starting a new agent — every action listed above.
  The Unassigned agents page and its contents need this switch **and** their
  own switch below both on; either alone leaves the group closed.
- **What's not gated (unchanged by this feature):** every other page in
  mdview — the project list, a project's markdown pages, search, the
  settings page itself, and the plain status/configuration views all remain
  reachable to anyone who can reach the server, exactly as before this
  feature. The project list in particular reveals no agent name and no
  working directory to any visitor; opening it, or opening the Unassigned
  agents page's presence marker, never adds anything to the registered
  project list either.
- **Off answers:** a page route (the Terminal tab, the Transcript tab, the
  Unassigned agents page) gets mdview's ordinary not-found page — the same
  page an unregistered project id gets, never a blank or typeless response.
  A route the client polls for data (a pane's screen or transcript, sending
  input, starting a pane) gets a not-found answer carrying a plain reason a
  script can read, so the client's own pollers get a reason rather than a
  page or an unreadable body.
- **The Unassigned group's own switch:** this group reaches every herdr pane
  on the host that sits outside every registered project's root —
  unrelated repositories, root shells, other people's agents — and has no
  boundary check of its own the way a project's panes do. It stays off until
  an operator deliberately turns it on; turning the terminal switch on alone
  never opens it.
- **How the switches are changed:** the settings page submits the terminal
  switches (and the notify destination and credential) as a request a page
  the operator merely has open elsewhere cannot forge — unlike an ordinary
  form submission, which a browser will send cross-site without the operator
  noticing, carrying whatever this daemon already trusts about that browser.
  With no authentication of its own, an ordinary form here would let any
  page the operator happens to be viewing flip these switches or overwrite
  the notify credential on their behalf; this one cannot be triggered that
  way.
- **The condition this rests on.** Nothing above proves who is asking — the
  terminal's safety depends entirely on the daemon's port being unreachable
  except through an authenticating front door placed in front of it (a
  reverse proxy, a VPN, a firewall rule — mdview provides none of these
  itself). If that front door is ever removed or misconfigured so the port
  becomes directly reachable, the terminal is unauthenticated remote code
  execution for anyone who can reach it: they can read and drive every
  running agent, start new ones, and — if the Unassigned switch is also on —
  read and drive every pane on the host. The terminal switch and the
  Unassigned switch are policy for an operator who already trusts everyone
  who can reach the port; neither is a substitute for that front door.
- **Afterwards:** turning the switch off immediately closes every gated
  route; turning it back on immediately reopens them — there is no
  credential to regenerate or session to re-establish either way.

### Guards that are not authentication

None of the guards below were touched by removing the terminal's
authentication, and each still holds:

- **Containment.** A pane outside a project's own root is refused on every
  action that names one — viewing, replying, reading its transcript, listing
  it (see Business Rules, Project scoping).
- **Creation-destination containment.** Starting a new pane or agent resolves
  its working directory automatically from this project's own boundary; a
  request can never name or influence the destination directly.
- **Fail-closed on an unconstructable boundary.** If a project's own
  containment boundary cannot be built at all, every action that needs it
  refuses cleanly — an empty pane list or a refused creation, never a crash
  and never a laxer check that lets something through.
- **Pane ids are never trusted from the URL.** A pane id named in a request
  is checked against the panes herdr actually reports for this project; an
  id for a real pane belonging to a different project, or to none, is
  refused exactly like one that does not exist.
- **Operator-authored argv only.** Starting a preset agent names only the
  preset's label; the command that actually runs is whatever an operator
  configured for that label in advance, and an unrecognized label is refused
  before herdr is ever called.
- **The input is bound.** A single request can carry only so many key
  presses; a request over that bound is refused before it reaches herdr.
- **Staged, not sent.** Typed text lands in the pane's composer without being
  submitted; sending it (pressing Enter) is a separate, explicit act.
- **Output is escaped before it becomes markup.** A pane's screen is
  translated into safe HTML — nothing in it is interpreted as markup, however
  it got onto that screen.
- **Named remedies, not raw errors.** A failure names what happened and, where
  there is one, the fix — never a bare stack trace, an internal path, or an
  unexplained status.
- **Typed text and named keys are never logged.** Nothing an operator types
  into a pane, or any key name sent to one, appears in this surface's own
  logging.
- **The notify credential stays write-only at rest.** Once saved, it is never
  read back into a page or an exported configuration — only a masked hint,
  and a failed save is reported as failed, never as saved.

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
  its own until this is turned on. If herdr keeps dying, restarts do not
  hammer it: each retry waits progressively longer than the last, up to a
  cap, and every backoff step is logged rather than only the first.
- **Notify on status change:** when switched on, mdview sends a notification
  message when a watched agent's status changes (this also needs a
  destination and a credential configured separately; the credential is
  never shown again in full once saved, and never appears in any viewed or
  exported configuration). Off by default; mdview makes no outbound call
  until this is turned on.
- Both duties, together with the notification destination and credential,
  are switched on and changed from the settings page, the same as every
  other setting on that same page (see Actors & Access) — no separate
  session or credential is needed to reach them. They take effect
  immediately without a restart.

## Actors & Access

One local operator per install, same as the rest of mdview. The terminal,
transcript, and agent-creation family carries **no authentication of its
own** — no token, no session, no login, no cookie. Reaching any of it
requires only that the terminal switch (and, for the Unassigned group, its
own switch too) is on — the same single condition that gates every route in
this family, described in full under "The terminal switch" above.

The settings page that hosts these switches is itself unauthenticated, like
every other page in mdview — reaching it is enough to view or change every
setting on it, this family's switches, the notify destination, and the
notify credential included. Nothing on that page is carved out behind a
session any more; see the Settings spec.

**This surface's safety rests entirely on something outside mdview.**
mdview proves nothing about who is asking. Whoever can reach the daemon's
port can drive this family exactly as the operator can, the moment the
terminal switch is on. What keeps that from being anyone on the internet, or
anyone on the operator's network, is a front door placed in front of the
port that does authenticate — a reverse proxy with its own login, a VPN, a
firewall rule restricting reachability to the operator's own machine — none
of which mdview provides. If that front door is ever removed, disabled, or
misconfigured so the port becomes reachable without it, the terminal is
unauthenticated remote code execution for anyone who reaches it: they can
read and drive every agent running under every registered project, start
new ones, and, if the Unassigned switch is also on, read and drive every
pane on the host, not only ones belonging to a registered project. This is
not a residual risk to be hardened later — it is the condition the entire
surface is built to run under, and it must be re-verified by whoever
operates mdview every time the network path to its port changes.

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
  Unassigned group, gated by the terminal switch and the group's own switch
  together. The registered project list is never changed by any of this:
  listing, or even opening, an agent never adds its project to the registry.
- **Tab always present (D6).** The Terminal tab (and Transcript tab) render
  on every registered project's page whether or not the terminal has ever
  been switched on or herdr is reachable; a missing herdr is a named state,
  never a hidden tab.
- **Off until switched on (D7).** The terminal switch, the Unassigned
  group's own switch, the "keep herdr running" duty, and the "notify on
  status change" duty are each independently off until an operator turns
  them on from the settings page; none of them changes mdview's behavior for
  an install that never visits that page.
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

- Terminal switched off: a page route (the Terminal tab, the Transcript tab,
  the Unassigned agents page) answers with mdview's ordinary not-found page;
  a route the client polls for data answers not-found with a plain,
  script-readable reason. Neither is a blank or typeless response.
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
- **The operator cannot scroll a pane from the dashboard.** herdr accepts no
  page-key name and offers no way to move a pane's own viewport; sending the
  raw escape a terminal would use leaves the pane's scroll position untouched.
  The screen box therefore scrolls itself over whatever tail herdr hands
  back, which is as far back as this surface can see. Reaching further would
  need herdr to grow the capability first.
- Confirmation against a real, running herdr (rather than a test double) is
  a manual check at UAT, not something automated coverage certifies.
- Mobile-specific layout for this surface, carried over as an idea from
  herdr-go's mobile-first design, is not settled.

## Visuals

No settled screenshot captured yet.

## Pointers (implementation)

- `crates/mdview/src/server.rs` — the routes themselves (`/p/:id/_terminal`,
  `/p/:id/_transcript`, their screen/input/keys/create children, and the
  `/_terminal/unassigned` family), each gated behind `terminal_family_enabled`
  alone (`unassigned_group_enabled` too, for the Unassigned family) — there is
  no authentication extractor left anywhere in this file; `project_panes`
  (agents joined to their pane's cwd, filtered by the D2 boundary) is what
  makes a plain shell invisible to listing and to every pane-scoped action
  (`project_and_verify_pane_in_boundary`, `project_pane_cwd_in_boundary`);
  `CreatePaneBody`/`CreateAgentBody` are the (empty / preset-label-only)
  request shapes a creation call actually sends; `update_terminal_config` is
  the one route that saves the switches plus the notify destination/credential,
  reachable with no gate at all so it stays available to turn the terminal
  switch back on — its body must be JSON, never a form (see "How the switches
  are changed" above); `terminal_disabled_page`/`terminal_disabled_json_404`
  are the two disabled-state answers, for page routes and polled data routes
  respectively.
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
