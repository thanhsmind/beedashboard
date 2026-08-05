# Agent Terminal — Context

**Feature slug:** agent-terminal
**Date:** 2026-08-05
**Shaping session:** complete
**Scope:** Deep
**Domain types:** SEE | CALL | RUN | READ

## Feature Boundary

mdview absorbs herdr-go: every registered project gains a Terminal tab that
lists the coding agents running under that project's root, shows each agent's
live screen, and sends typed replies and keys back to it — plus the background
duties herdr-go carried (keeping herdr alive, notifying on status change,
reading the agent's own transcript), all off until switched on. herdr-go stops
being a separate product. It ends at the herdr socket: mdview never owns a PTY
of its own and never replaces herdr itself.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | mdview absorbs herdr-go whole — the terminal surface, agent status, the herdr supervisor, Telegram notification and the transcript channel all move into mdview, and herdr-go stops being a separate product. | Chosen over a terminal-only lift, so there is one agent cockpit rather than two products sharing one herdr socket. |
| D2 | The terminal lives inside a registered project at `/p/:id/_terminal`, and lists only the herdr panes whose working directory sits under that project's `root_path`. | mdview already owns the project registry, so the project is the frame; the ask was pick-a-project-then-pick-a-terminal, not one flat global list. |
| D3 | The terminal is read **and** write: it renders the polled screen and sends both free text and named keys back to the agent's pane. | Replying from another device is the point; an observe-only pane would not replace herdr-go, which D1 retires. |
| D4 | Authentication gates **only** the terminal routes. Every other mdview route stays unauthenticated exactly as today. | Sending keystrokes into a live coding agent is the only new capability that is dangerous on a LAN-bound `0.0.0.0` server. Gating the whole app would break the open-the-link and MCP `doc_viewer` URL flows mdview depends on. |
| D5 | Herdr panes whose working directory is under no registered project appear in an **Unassigned** group on the project list page, viewable as terminals but belonging to no project. The registry is never auto-populated from a pane's cwd. | Keeps registration a deliberate user act while guaranteeing no running agent silently disappears from the cockpit. |
| D6 | The Terminal tab is always present on a project page. When the herdr socket does not answer, the tab renders an explicit "herdr is not running" state naming the remedy, instead of hiding itself. | A hidden tab makes the capability invisible and unexplained; a named reason teaches the user what is missing. |
| D7 | The herdr supervisor and Telegram notification are both opt-in, off by default, toggled on the settings page. mdview spawns no process and makes no outbound network call until the user turns them on. | mdview's baseline product is a local markdown viewer; silently spawning a herdr server or calling Telegram would be an unrequested behavior change for every existing user. |
| D8 | The cockpit can **start** new agents: herdr-go's pane creation, `agent.start` and the configured agent presets all move into mdview, so a user can spawn a coding agent in a project from the terminal surface. | D1 retires herdr-go, so anything only herdr-go could do would otherwise be lost. Accepted with the added auth weight in view. |
| D9 | The transcript is a **second tab beside Terminal** on a project page, not a toggle inside the terminal frame. | The polled screen and the gap-free semantic activity log answer different questions and lose data when collapsed into one frame. |
| D10 | The terminal token is generated, shown and rotated on mdview's **settings page**, at the moment the terminal surface is switched on. | Keeps issuance beside the D7 opt-in switches rather than splitting setup between the settings page and `doctor`. See the Known Risk below. |

### Known Risk Accepted With D10

D4 exists to stop anyone on the LAN from typing into a running agent. D10 puts
the token on the settings page, which D4 leaves unauthenticated — so anyone who
can reach mdview can read the token and the gate is void in practice. The user
chose D10 with that stated. Planning must therefore treat one of these as part
of the work, and name which:

- restrict the token's *display* (not the settings page) to loopback requests, or
- show the token in full exactly once at creation and only its last characters
  afterward, or
- an equivalent the plan argues for explicitly.

Shipping D4 and D10 together with no such measure is a defect, not a trade-off.

### Agent's Discretion

- Cross-platform mechanics of reaching herdr (Unix domain socket vs Windows
  named pipe) carry over as herdr-go already solved them; no new product
  decision is implied.
- Wording of the "herdr is not running" state (D6), as long as it names a
  remedy rather than only a failure.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Agent | One coding agent herdr is running. What the user recognises and picks. |
| Pane | herdr's addressable unit behind an agent — the thing screens are read from and input is sent to. |
| Screen | The current visible ANSI content of a pane, polled as a snapshot. Not a live stream. |
| Unassigned | The group holding panes whose working directory is under no registered project root (D5). |
| Terminal tab | The per-project surface at `/p/:id/_terminal` (D2, D6). |
| Transcript | The agent's own on-disk JSONL session log, tailed gap-free. A second observation channel beside the screen, on its own tab (D9). |

## Specific Ideas And References

- The user's framing: "the project directory already exists, so adding
  pick-a-project-then-pick-a-terminal is the nice shape." That is D2 —
  the project list is the entry point, not a new flat agent index.

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `/home/thanhsmind/projects/goglbe/herdr-go/src/herdr/socket.rs` — the whole
  herdr client: newline-JSON request/response over a Unix socket (Windows named
  pipe via `interprocess`), one request per connection. Carries the methods this
  feature needs: `session.snapshot`, `pane.read`, `pane.send_input`,
  `pane.send_keys`, `tab.create`, `agent.start`. Depends only on
  tokio/serde/async-trait — no axum — so it lifts into `mdview-core` cleanly.
- `/home/thanhsmind/projects/goglbe/herdr-go/src/herdr/wire.rs` — the wire DTOs
  (`Agent`, `Snapshot`) the socket client returns.
- `/home/thanhsmind/projects/goglbe/herdr-go/src/herdr/fake.rs` — in-memory
  `Herdr` implementation; the existing seam for tests without a live herdr.
- `/home/thanhsmind/projects/goglbe/herdr-go/src/web/screen.rs` — the read/reply
  handlers and their body shapes (`ScreenBody { text, revision }`,
  `ReplyBody { text, submit }`, `KeysBody { keys }`).
- `/home/thanhsmind/projects/goglbe/herdr-go/src/web/auth.rs` — constant-time
  token compare, random session id, `HttpOnly; SameSite=Strict` cookie,
  opaque-404 on failure. The mechanism D4 applies to the terminal routes.
- `/home/thanhsmind/projects/goglbe/herdr-go/web/src/views/terminal.ts` — the
  client that polls every 1500 ms and renders ANSI into xterm.js as a static
  snapshot with the cursor hidden.
- `/home/thanhsmind/projects/goglbe/herdr-go/src/security/paths.rs` — the
  fail-closed 7-step path boundary used to keep pane cwds inside allowed roots.

### Established Patterns

- Server-rendered HTML built from plain `format!` strings with a local `esc()`
  helper — `crates/mdview/src/views.rs:1-40`. Every mdview page follows it; the
  terminal page must too.
- Vendored JS served from a `const` and its own route, the way
  `mermaid.min.js` is (`crates/mdview/src/server.rs:264-297`). There is **no**
  frontend build step anywhere in the workspace.
- A `tokio::sync::broadcast` channel in `AppState` fanned out over the existing
  `/ws` route (`crates/mdview/src/server.rs:686-706`) — the only live-push
  mechanism the codebase has today.
- Read-only snapshot readers that degrade to partial data instead of erroring —
  `crates/mdview-core/src/bee.rs:481` (`read_snapshot`).

### Integration Points

- `crates/mdview/src/server.rs:93-113` — the route table the terminal routes
  join, alongside the existing `/p/:id/_bee*` family.
- `crates/mdview-core/src/domain.rs:8-15` — the `Project` struct
  (`id`, `name`, `root_path`, …). `root_path` is what D2 matches pane cwds
  against; D5 means the registry itself is not extended by this feature.
- `crates/mdview-core/src/repository.rs` — the SQLite registry at
  `~/.mdview/registry.db`.
- `crates/mdview-core/src/config.rs:8-58` — the `Config` tree that gains the
  D7 opt-in switches; settings page at `crates/mdview/src/server.rs` `/settings`.
- `crates/mdview-core/src/config.rs:64-67` and `crates/mdview/src/server.rs:69-77`
  — where mdview declares and warns that it has no auth. D4 makes this statement
  partially untrue and both sites must be corrected.

## Canonical References

- `docs/specs/system-overview.md` — what mdview is; absorbing a second product
  changes it.
- `docs/specs/settings.md` — states plainly that there is no authentication;
  D4 and D7 both land here.
- `docs/specs/reading-map.md` — the index a new "agent terminal" area is added to.
- `docs/specs/bee-cockpit.md` — the closest existing surface in shape and
  placement (`/p/:id/_bee`).
- `/home/thanhsmind/projects/goglbe/herdr-go/src/web/mod.rs:1-5` — herdr-go's own
  statement that there is no live WebSocket terminal, only observe + reply.
- herdr-go decision `675fc93a` (cited at `src/web/mod.rs:5` and
  `src/web/screen.rs:1-4`) — why the terminal polls ANSI snapshots instead of
  streaming a PTY: herdr's socket API has no PTY-sizing primitive.

## Outstanding Questions

### Resolve Before Planning

None. Every product decision this feature needed is locked in D1–D10.

### Deferred To Planning

- [ ] Which measure closes the D10 token-exposure gap (see **Known Risk**). The
      choice is technical; that *some* measure ships is not optional.
- [ ] Whether agent creation (D8) is gated by the same token as observe/reply,
      or held to something stronger given it starts processes.

- [ ] How xterm.js enters a workspace with no frontend build step — vendored as
      a `const` asset like `mermaid.min.js`, or a build step introduced. Weigh
      bundle size against the precedent.
- [ ] Whether the 1500 ms screen poll stays HTTP polling or moves onto the
      existing `/ws` broadcast channel. Note that herdr's socket is
      request/response only, so a push transport changes the browser leg, not
      the herdr leg.
- [ ] Whether the herdr client lands in `mdview-core` (framework-free, matching
      `bee.rs`) or in `crates/mdview`, and what it does to build time and the
      dependency set — `rusqlite` and `interprocess` both link native code.
- [ ] Whether herdr-go's `allowed_roots` boundary is still needed once mdview's
      registered project roots define the visible set (D2/D5), or whether the
      two must both hold.
- [ ] What the Windows leg costs, given mdview already carries a cross-platform
      flag and herdr-go reaches herdr over a named pipe there.

## Deferred Ideas

- herdr-go's standalone service management (`herdr-go service start|stop|…`,
  systemd unit / launchd / Scheduled Task) — mdview already has its own daemon
  lifecycle (`docs/specs/daemon.md`); reconciling the two is its own work, not
  this feature's.
- herdr-go's Cloudflare Access JWT fallback (`Cf-Access-Jwt-Assertion`) — D4
  settles the token mechanism; a second auth path is separate work.
- Mobile-specific layout work carried over from herdr-go's mobile-first UI —
  worth doing, but not a decision this feature had to lock.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Nothing blocks planning. Read the **Known Risk Accepted With D10** section before
shaping the auth work — it is the one place where two locked decisions pull against
each other and the plan must say how it resolves them. Planning's Gate 2 shape stage
and reviewing use locked decisions for coverage and UAT.
