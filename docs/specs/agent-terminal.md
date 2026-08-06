---
area: agent-terminal
updated: 2026-08-06
sources: [agent-terminal]
decisions: [D1, D2, D3, D4, D5, D6, D7, D8, D9, D10]
coverage: partial
---

# Spec: Agent terminal

mdview absorbs herdr-go, the standalone mobile-first gateway that watched and
replied to coding agents running under [herdr](https://github.com/ogulcancelik/herdr).
herdr-go is retired; this is its successor inside mdview. Every registered
project gains a Terminal tab (watch and reply to the agents running under
that project) and a Transcript tab (each agent's own activity log), plus two
background duties herdr-go used to carry, both off until an operator switches
them on. mdview never owns a terminal of its own — it always talks to a
running herdr, the same way herdr-go did.

Technology-agnostic: this describes behavior and rules, not the Rust that
implements them. Code entry points are listed in `reading-map.md`.

## Entry Points & Triggers

- A registered project's page → a "Terminal" tab, always present, alongside
  the existing tabs, whether or not the terminal has ever been switched on.
- Opening the Terminal tab → lists every coding agent ("pane") herdr is
  running whose working directory sits under this project's root, each with
  its live-polled screen and a control to reply.
- Opening the "Transcript" tab beside it → the same set of agents, each
  showing its own on-disk session log instead of a screen.
- A pane running outside every registered project's root → not listed on any
  project's tab; instead a card on the project list page, "Unassigned
  agents," opens a page listing exactly those panes.
- The settings page → where the terminal is switched on, where its access
  token is generated and rotated, and where the two background duties are
  switched on.

## Data Dictionary

| # | Element | Meaning | Values |
|---|---|---|---|
| 1 | Agent (pane) | One coding agent herdr is running, addressed by its own id | id, working directory, current screen |
| 2 | Screen | The agent's current visible terminal contents, rendered with colour | polled roughly every 1.5 seconds — a snapshot redrawn on each poll, not a live feed |
| 3 | Transcript | The agent's own on-disk session log, read directly rather than through herdr | a gap-free running record of the agent's activity, independent of the screen poll |
| 4 | Access token | The one credential that unlocks the terminal, transcript, and agent-creation routes | generated/rotated on the settings page; shown in full exactly once, at the moment it is generated or rotated — every later view of the settings page shows only its last few characters |
| 5 | Unassigned agents | Agents whose working directory is outside every registered project's root | listed on their own page, gated the same as any project's agents |
| 6 | Keep herdr running | Opt-in duty: mdview keeps the herdr process alive on the operator's behalf | on / off, off by default |
| 7 | Notify on status change | Opt-in duty: mdview sends a Telegram message when a watched agent's status changes | on / off, off by default |

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

- **Triggers:** typing free text and submitting it, or sending a named key
  (for example Enter, an arrow key, Ctrl+C), from a listed agent's pane.
- **What it does:** the text or key is sent to that agent exactly as if typed
  at its own terminal.
- **Afterwards:** the agent's next screen poll reflects whatever it did with
  the input.

### Starting a new agent

- **Triggers:** using the Terminal tab's creation control — picking one of
  the presets an operator configured in advance, or naming a plain working
  directory.
- **What it does:** starts a new coding agent under this project; it appears
  in the pane list on the next poll.
- **Blocked when:** the destination falls outside this project's own root, or
  the requested preset is not one of the configured ones.

### Viewing the transcript

- **Triggers:** opening a registered project's Transcript tab with a valid
  session.
- **What it shows:** the same set of agents as the Terminal tab, each showing
  its own session log instead of a screen.
- **Afterwards:** the operator can see what an agent did even for output that
  has since scrolled off or been cleared — something the polled screen alone
  would lose. The screen and the transcript answer different questions and
  are kept as separate tabs rather than merged into one.

### Unlocking the terminal (the token boundary)

- **Triggers:** entering the access token on the settings page.
- **What it does:** presenting the correct token starts a session; every
  terminal, transcript, and agent-creation request is refused unless it
  carries a valid session for the token currently in effect.
- **What's gated:** the Terminal tab and its screen, the Transcript tab and
  replying and sending keys, starting a new agent, and the Unassigned agents
  page and its contents.
- **What's not gated (unchanged by this feature):** every other page and
  route in mdview — the project list, a project's markdown pages, search,
  the settings page itself, and the JSON status/config endpoints all remain
  reachable to anyone who can reach the server, exactly as before this
  feature. A request that lacks a valid session for a gated route is
  answered exactly as an address that was never routed at all would be —
  there is nothing that distinguishes "wrong token" from "no such page."
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
- **Notify on status change:** when switched on, mdview sends a Telegram
  message when a watched agent's status changes (this also needs a
  destination and a credential configured separately). Off by default;
  mdview makes no outbound network call until this is turned on.
- Both are switched on from the settings page, beside the access token, and
  take effect immediately without a restart.

## Actors & Access

One local operator per install, same as the rest of mdview. Unlike every
other surface in mdview, this one is gated: reaching the terminal,
transcript, or agent-creation family requires the session that presenting
the access token establishes (see the token boundary above). The settings
page where that token is generated and rotated is itself unauthenticated,
like the rest of mdview outside this family — reaching it is enough to view
or change every other setting, and, once a token exists, to attempt logging
in with it.

## Business Rules

- **Project scoping (D2).** An agent's working directory decides which
  project, if any, lists it; an agent under no registered project's root
  appears only in the Unassigned group, never silently on some other
  project's tab.
- **Nothing lost (D5).** An agent whose working directory is outside every
  registered project is never dropped from view — it always appears, in the
  Unassigned group, behind the same gate as everything else.
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

## Edge Cases Settled

- Terminal switched off: the tab's route answers exactly as an unrouted
  address would, even for a visitor who already holds a valid session.
- No herdr socket to reach: every affected view degrades to the named
  "herdr is not running" state rather than an error page or a blank screen.
- An agent whose working directory is outside every registered project: it
  is never dropped — it appears in the Unassigned group instead.

## Open Gaps

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
  terminal-enabled switch before it does anything else.
- `crates/mdview/src/terminal_auth.rs` — the token/session mechanism: token
  storage, reveal-once rotation, and the opaque-404 answer a failed or
  missing session gets.
- `crates/mdview/src/herdr/` — the client that talks to a running herdr over
  its socket (or named pipe on Windows).
- `crates/mdview/src/supervisor.rs`, `crates/mdview/src/notify/` — the two
  opt-in background duties.
- `crates/mdview/src/views.rs` — the tab pages, screen/transcript rendering,
  and the herdr-down state's wording.
- `crates/mdview-core/src/config.rs` — the D7 switches and the agent-create
  presets in `Config`.
- `crates/mdview-core/src/transcript.rs` — reading an agent's own on-disk
  session log.
- `crates/mdview-core/src/paths_boundary.rs` — the containment check that
  scopes panes to a project's root.
- `crates/mdview-core/src/notify_store.rs` — the notification outbox used
  only when the notify duty is on.
- `crates/mdview-core/src/ansi.rs` — translating a raw screen into safe,
  coloured HTML.
