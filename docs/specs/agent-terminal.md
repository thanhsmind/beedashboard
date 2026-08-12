---
area: agent-terminal
updated: 2026-08-13
sources: [agent-terminal, terminal-open-access, scroll-fab-follow]
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
- Opening the Terminal tab → a strip naming every session herdr is running
  in this project's folder, one entry each, and beneath it the one session
  that entry selects: its live-polled screen and a control to reply, plus a
  control to start a new agent. Each strip entry is its own address, so a
  single session can be opened, sent, or bookmarked on its own.
- Opening the "Transcript" tab beside it → the same strip over the same
  sessions, the selected one showing its activity log instead of a screen.
- A session whose folder sits outside every registered project's root → not
  listed on any project's tab; instead a card on the project list
  page, "Unassigned agents," opens a page listing exactly those agents. The
  card itself carries no agent name and no working directory — it is a bare
  presence marker, shown only while both the terminal switch and the
  Unassigned group's own switch are on (see Business Rules); with either
  off, the card does not appear at all.
- The settings page → where the terminal switch, the Unassigned group's own
  switch, and the two background duties are turned on, alongside every
  other, unrelated mdview setting — see Actors & Access for what this
  surface's safety now rests on.
- A fixed tab on the Terminal or Transcript page's edge → a slide-in drawer
  listing every agent across every registered project, for switching
  directly to any of them without first navigating to that project's own
  page.

## Data Dictionary

| # | Element | Meaning | Values |
|---|---|---|---|
| 1 | Agent | One coding agent herdr is running, addressed by its own id | id, folder (via its pane), status, the program it runs, current screen |
| 2 | Pane | The addressable session, listed whether or not an agent runs inside it: every agent has exactly one, and a session opened with no agent started in it is listed too, as a shell | id, workspace and tab it sits in, launch folder, live folder, status, an agent when one is attached |
| 2a | Status | What a listed session is doing, shown as a named dot on its entry | working, blocked, done, idle, unknown, or shell for a session with no agent; the first three each read as their own colour, the rest as the quiet one |
| 3 | Screen | The agent's recent terminal contents, rendered with colour | a snapshot redrawn on each poll, not a live feed; a bounded tail of up to 200 lines of the pane's own scrollback rather than only the rows currently on screen, so a plain shell shows work that has already scrolled past, while an agent that redraws a full-screen interface has no scrollback to give and shows exactly its current frame; shown at the full height of one pane frame with its lines unwrapped, the box scrolling in both directions rather than re-flowing the frame; rendered in full 24-bit colour rather than a limited palette, since agents' own output overwhelmingly uses colour beyond the basic 16 or 256-colour sets |
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
- **What it shows:** a strip naming every session in this project's folder —
  each entry carrying, in order, the workspace and tab it sits in, its
  status dot, and the program it runs — and beneath it exactly one of them:
  its screen rendered as coloured text and a control to reply. That identity
  is printed once, on the strip entry itself; the screen and transcript
  views beneath it carry no heading of their own repeating it, since a
  second copy only pushes the content down a handset's viewport. Sessions
  with no agent are named as shells and are listed like any other. A session
  belonging to a different project, or to none, never appears here.
- **Starting another session:** the control that starts one shares the strip's
  own row, at that row's right end, so the list of sessions and the way to add
  one read as a single band rather than two stacked controls.
- **Which one is shown:** the entry the address names. Opening the tab
  without naming one shows the session the operator is currently focused on
  when it belongs to this project, and otherwise the first in the strip, so
  the tab always opens on something.
- **How the screen renders:** a path to one of the project's own markdown
  docs, wherever it appears in the screen or the transcript, is shown as a
  clickable link to that doc. The monospaced type used for the screen ships
  with mdview itself, so opening the tab never depends on reaching an
  outside font service, and it covers the box-drawing characters, Vietnamese
  text, and other characters an agent's screen may use. A run of box-drawing
  lines an agent draws — a table or a frame — always keeps its original
  layout and scrolls sideways on its own rather than being wrapped and
  ruined; a single non-box line inside such a run is absorbed into it
  rather than splitting it, and output containing no box-drawing at all
  renders exactly as it always did.
- **On a narrow screen (a viewport under 720 pixels wide, the same threshold
  the rest of the browser chrome uses):** the page collapses to two rows: the project's own
  navigation moves into the top bar beside the brand, and the strip below it
  carries the sessions plus the "new session" control at its own row's right
  end. The reading spacing above the screen is tightened so the screen
  itself starts within the first view on a phone rather than below the
  fold. The strip itself costs one line instead of several: the entry for
  the session being viewed stays on the line; every other entry, together
  with the controls that start a new session, moves behind one menu control
  beside it. Opening that menu lists them all — the viewed session included,
  in its place among the others — so nothing is reachable only on a wide
  screen. With no session to switch between there is no menu at all, and the
  creation controls stand on their own. The menu opens and closes without
  scripting, the same way the top bar's does (see the Web interface spec).
  The screen's own prose text wraps to fit the narrower width here instead
  of requiring the two-way scrolling described above, at a type size
  measured to stay readable and refitted automatically whenever the pane's
  width changes; a run of box-drawing lines still keeps its unwrapped,
  sideways-scrolling layout even on a narrow screen. Below the screen, the
  named reply keys and the pane's own arrow keys share a single row rather
  than stacking across two, keeping as much vertical space as possible for
  the screen itself. Touch scrolling inside the screen stays contained to
  it — it does not also trigger the browser's own pull-to-refresh or
  page-navigation gesture.
- **Afterwards:** the operator sees exactly the sessions that belong to this
  project, and has an address for each one on its own.

### Reaching a pane's older output

- **Triggers:** pressing a listed agent's Older, Newer, or Live control
  while the terminal switch is on.
- **What it does:** Older steps one step further back than the previous
  press reached; Newer steps one step forward toward the live view and is
  unavailable once already there; Live jumps straight back to the current
  live view in one press regardless of how many steps were taken. The three
  are separate, individually labelled controls, so an assistive reader
  never confuses them with the pane's own arrow keys, and each meets the
  same 44px touch-target minimum as the pane's other controls. They sit
  stacked together against the screen's right edge, and they follow the
  reader: while any part of the pane's screen is in view the stack stays at
  the lower edge of the viewport, so a long screen never leaves the controls
  scrolled off below. Once the screen's own bottom comes into view the stack
  stops there rather than travelling past it — it never leaves the screen's
  frame, never covers the reply controls beneath the screen, and stays clear
  of a phone's own safe-area inset, regardless of the pane's height or the
  window's width. While an
  operator has stepped back this way, the pane's normal live refresh stops
  updating that view, so it is never overwritten out from under them.
- **How stepping stays cheap:** the surface remembers how far back each pane
  has already gone and moves only the extra step a press asks for, rather
  than replaying every step from scratch each time. Landing on Live always
  works, even for a pane nothing was remembered for. A pane left stepped
  back with no further request for more than 90 seconds is swept back to
  live automatically the next time any pane's screen is polled. Opening a
  session's live view always restores a previously stepped-back pane to
  live first. If the screen has materially changed underneath while stepped
  back (a change confined to the status footer does not count), the next
  step falls back once to a full restore-and-replay instead of an
  incomplete jump, and what was remembered for that pane is cleared. Each
  pane's stepping is handled one request at a time, so overlapping requests
  for the same pane can never race each other.
- **Afterwards:** pressing Live returns the pane to its current live view
  and lets the normal refresh resume; nothing about the pane's connection,
  or the operator's ability to reply, is affected by having stepped back
  first. Leaving the page (navigating away, or the tab going into the
  background) sends a best-effort request to restore the pane to live, so
  it is not left stepped back for the next visit.

### Replying to an agent

- **Triggers:** typing free text, sending a named key (for example Enter, an
  arrow key, Ctrl+C), or reading the current screen or the transcript, from
  a listed agent's pane, while the terminal switch is on.
- **What it does:** typed text can be staged into the agent's pane without
  being sent — submitting it (pressing Enter) is a separate act the operator
  chooses explicitly; a named key is sent immediately. The keys offered are
  the four arrows, then Enter, Escape, Tab and an interrupt (Ctrl+C) — the
  last of these stops whatever the agent is running and, like every other
  named key, acts on the first press with no confirmation step. Every one of these
  actions — reading the screen, sending text, sending a key, reading the
  transcript — is refused unless the target agent already belongs to this
  project, exactly like viewing the terminal itself.
- **Afterwards:** the agent's next screen poll reflects whatever it did with
  the input.

### Attaching images to a reply

- **Triggers:** picking image files with the composer's attach control,
  dragging them onto the composer, or pasting an image from the clipboard
  into the reply box — several images at a time — on a project's per-pane
  terminal page, while the terminal switch is on.
- **What it does:** each image is stored on the machine in a per-pane
  holding area owned by the operator's own user account, outside the
  project's folder, under a name the server invents — the sender's file
  name never becomes part of where it lands. The composer shows each stored
  image as a removable chip. Sending then delivers ONE submitted message to
  the agent: the typed prompt (when any), then the stored images' locations,
  space-separated — a location containing whitespace is double-quoted so it
  survives as one word. A successful send clears the chips; a chip removed
  before sending never appears in the message. The composer's own keys are
  unchanged: plain Enter still opens a new line, and sending is still the
  explicit act it always was.
- **Blocked when:** the file is not one of the accepted image kinds (PNG,
  JPEG, GIF, WebP — judged by its actual content, not just its declared
  kind, and SVG is deliberately excluded as scriptable); the file exceeds
  10 MB (refused in the page before any upload starts, and again by the
  server); or the pane already holds 32 stored images. Every refusal is
  shown beside the composer in words and stores nothing.
- **Afterwards:** stored images older than a day are swept out of the
  pane's holding area the next time an upload arrives, so the 32-image
  ceiling is a working-set bound, not a lifetime one. What the agent does
  with the locations is its own affair.

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
- **What it shows:** the same strip of sessions as the Terminal tab, the
  selected one showing its own session log instead of a screen, and the same
  rule picking it when the address names none. A session that has not written
  anything yet reports plainly that no transcript is available yet, rather
  than showing an empty frame that could be mistaken for "caught up." A
  session claimed by this project through its live folder rather than its
  launch folder reads its log from that live folder, which is why one can
  legitimately report nothing while the same session's log is full on the
  project it was launched in.
- **Afterwards:** the operator can see what an agent did even for output that
  has since scrolled off or been cleared — something the polled screen alone
  would lose. The screen and the transcript answer different questions and
  are kept as separate tabs rather than merged into one.

### Switching between agents

- **Triggers:** opening the fixed edge tab on a Terminal or Transcript page
  while the terminal switch is on.
- **What it shows:** a slide-in drawer listing every agent pane across every
  registered project — not only the ones belonging to the project currently
  open — plus the Unassigned group's own agents when the Unassigned group's
  own switch is also on (see The terminal switch); with that switch off, the
  drawer lists every registered project's agents and simply leaves that
  group out, the same as everywhere else it is gated. Plain shells are left
  out; this list is agents only. A pane claimed by more than one project
  (see Business Rules, Project scoping) is listed once, under the first
  project that claims it, never twice. Panes are grouped under a heading for
  each status — working, waiting on the operator, done, or idle — and each
  row opens straight onto that pane's own terminal page.
- **When it refreshes:** the drawer's contents are fetched only while it is
  open, on a short repeating interval; closing it stops the refresh, so
  having the drawer available costs nothing while it isn't in use.
- **Afterwards:** the operator reaches any agent in any project directly
  from wherever they already are, without first navigating to that
  project's own page.

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
  the Transcript tab, the Agents drawer, and starting a new agent — every
  action listed above. The Unassigned agents page and its contents, and the
  Unassigned group's entries in the Agents drawer, need this switch **and**
  their own switch below both on; either alone leaves the group closed.
- **What's not gated (unchanged by this feature):** every other page in
  mdview — the project list, a project's markdown pages, search, the
  settings page itself, and the plain status/configuration views all remain
  reachable to anyone who can reach the server, exactly as before this
  feature. The project list reveals no agent name and no working directory to
  any visitor; opening it, or opening the Unassigned agents page's presence
  marker, never adds anything to the registered project list either. It does,
  however, mark each project row with its own sessions — their state and the
  program each runs — and that marking follows this switch: off, the list
  behaves as though the feature did not exist and asks this host nothing (see
  the Web interface spec).
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
- **Colour stays hex-only.** The screen's colour rendering, including its
  full 24-bit colour support, never places anything but hex digits into
  that markup, so richer colour never opens a path around the escaping
  guarantee above.
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
- A screen poll that fails for any other reason (a dropped connection, a
  timeout, and so on) is treated differently: the last screen stays on
  view, marked with a "reconnecting…" indicator, rather than being replaced
  by the down message — a momentary blip never wipes out what the operator
  was looking at. Once polling succeeds again, the screen always repaints
  fresh rather than getting stuck showing stale content.

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

- **Project scoping (D2).** A session's folder decides which project, if
  any, lists it, and a session carries two: the folder it was launched in
  and the folder its foreground work is in right now. Either one inside the
  project's root claims the session; the launch folder is asked first, so a
  session already claimed by the project it started in never moves. A
  session whose foreground work has walked into another registered project
  is listed by that project too — being claimed twice is allowed, and each
  project answers for it under its own boundary. The live folder is only
  ever consulted on a machine that reports one.
  This boundary is enforced on every action that names a session — viewing
  its screen, sending text, sending keys, reading its transcript, listing
  it, and choosing where a new agent starts — never only on listing and
  creation. Both folders are put through the identical check: either one
  that escapes the project's root, by walking up through parent directories
  or by following a symbolic link out of it, is refused.
- **Being listed is being reachable.** The same list that decides what the
  operator sees is what decides what may be read from and typed into. So
  every widening of it is a widening of reach: a shell session inside the
  project's folder, invisible before, is now fully readable and writable by
  anyone who can open the page, and so is a session that has merely walked
  its foreground work into the folder. The terminal has no credential of its
  own (see The terminal switch), so the switch and this boundary are the
  whole of what stands there.
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
- **Attachments are scoped like everything else.** The image attach surface
  exists only on a project's own terminal pages and obeys the same two
  guards as replying — the terminal switch and the pane belonging to the
  project. The Unassigned group's page deliberately renders no attach
  control, because that surface has no project scope to validate against.
  The holding area is bounded three ways — accepted kinds judged by
  content, a per-file size ceiling, a per-pane count ceiling with a daily
  sweep — and lives outside every project folder so attaching never
  dirties a repository.
- **An agent is not its pane.** "Agent" is the coding agent herdr is
  running; "pane" is the session it runs inside. Every agent has exactly one
  pane, but the reverse does not hold — a plain shell opened with no agent
  started in it is a pane with no agent. The project's own list is about
  panes, so a shell in the project's folder is listed like anything else,
  named as a shell rather than borrowing an agent's words. The Unassigned
  group is still about agents only, so a shell outside every registered
  project remains invisible — see Open Gaps.

## Edge Cases Settled

- Terminal switched off: a page route (the Terminal tab, the Transcript tab,
  the Unassigned agents page) answers with mdview's ordinary not-found page;
  a route the client polls for data answers not-found with a plain,
  script-readable reason. Neither is a blank or typeless response.
- Herdr not reachable: every affected view degrades to the named "herdr is
  not running" state rather than an error page or a blank screen.
- An agent whose folder is outside every registered project: it is never
  dropped — it appears in the Unassigned group instead.
- A session reporting neither folder: excluded from every project's list, and
  every action naming it is refused.
- A session named in an address but not in this project's list: the ordinary
  not-found page, and the answer never repeats the id or folder it refused.
- A project with no sessions at all: its own named empty state, not a
  not-found.
- An agent with no transcript written yet: answered as a named, successful
  "nothing yet" state, never as an empty list indistinguishable from "caught
  up, nothing new."
- A screen poll failing for a reason other than herdr being down: the last
  screen stays on view with a "reconnecting…" indicator, never replaced by
  the down message.
- A pane's screen diverging while stepped back to older output: the next
  step falls back once to a full restore-and-replay, and what was
  remembered about that pane's position is cleared.

## Open Gaps

- **A plain shell outside every project is still not addressable.** The
  project's own list now covers shells, so one inside a registered folder is
  visible and reachable. The Unassigned group is not: it still lists agents
  only, so a shell under no registered project appears nowhere. D5's wording
  says "panes", so the group is narrower than its own decision. This was left
  deliberately: widening that group would expose every shell on the machine
  through a surface that carries no containment check of its own. Closing it
  properly is the user's call.
- Confirmation against a real, running herdr (rather than a test double) is
  a manual check at UAT, not something automated coverage certifies.

## Visuals

No settled screenshot captured yet.

## Pointers (implementation)

- `crates/mdview/src/server.rs` — the image attach route
  (`POST /p/:id/_terminal/:pane_id/attach`): raw-body upload, declared-MIME
  allowlist + magic-byte sniff, explicit 10 MB check answering in the
  terminal JSON error shape under a higher `DefaultBodyLimit`, 24h mtime
  prune then 32-file cap, storage at `$XDG_RUNTIME_DIR/mdview-attach` else
  `~/.cache/mdview/attach` with `[A-Za-z0-9-]`-sanitized project/pane
  segments and `rand`-hex leaf names; client wiring in
  `crates/mdview/assets/app.js` (composer IIFE: upload/chips/composeMessage)
  and gated markup via `pane_cards(..., attach)` in `crates/mdview/src/views.rs`.
- `crates/mdview/src/server.rs` — the routes themselves (`/p/:id/_terminal`,
  `/p/:id/_transcript`, their per-pane `pane/:pane_id` pages, their
  screen/input/keys/create children, and the `/_terminal/unassigned` family),
  each gated behind `terminal_family_enabled`
  alone (`unassigned_group_enabled` too, for the Unassigned family) — there is
  no authentication extractor left anywhere in this file; `project_panes`
  iterates `snapshot.panes` and accepts a pane whose `cwd` validates against
  the D2 boundary, falling back to `foreground_cwd` when `cwd` is absent or
  refused, joining the agent afterwards by `pane_id` — it is the single
  membership decision behind every pane-scoped action
  (`project_and_verify_pane_in_boundary`, `project_pane_cwd_in_boundary`),
  and the path it returns is the validated one, which is what keeps a
  transcript read inside the root; `unassigned_panes` subtracts that set but
  keeps its own agent-only output loop, so it can only ever shrink;
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
- `crates/mdview/src/views.rs` — the tab pages, the pane tab strip and its
  per-pane hrefs, the `<workspace> · <tab>` identity and the `.fg-status`
  pill that carries a session's status, screen/transcript rendering, the
  44px arrow target (`.term-controls > .term-keys button`), the herdr-down
  state's wording, and `project_list_page`'s presence-only Unassigned card.
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
