---
artifact_contract: bee-plan/v1
mode: high-risk
approved_gate2: 2026-08-05
---

# Plan: Agent Terminal

Mode: `high-risk` — 6 risk flags: auth, audit/security, external systems,
public contracts, cross-platform, multi-domain.
Why this is the least workflow that protects the work: the feature hands a
network-reachable HTTP surface the power to type into a running coding agent and
to start new ones, in a codebase that documents itself as having no
authentication — the auth flag alone is a hard gate, and no lighter lane may
carry it.

## Requirements (from CONTEXT.md)

- **D1** — mdview absorbs herdr-go whole: terminal, agent status, supervisor,
  Telegram notification, transcript. herdr-go stops being a separate product.
- **D2** — The terminal lives at `/p/:id/_terminal` and lists only the herdr
  panes whose working directory sits under that project's `root_path`.
- **D3** — Read and write: render the polled screen, send free text and named
  keys back.
- **D4** — Authentication gates only the terminal routes; every other mdview
  route stays unauthenticated.
- **D5** — Panes under no registered project appear in an **Unassigned** group
  on the project list page. The registry is never auto-populated from a cwd.
- **D6** — The Terminal tab is always present; when herdr does not answer it
  renders an explicit "herdr is not running" state naming the remedy.
- **D7** — Supervisor and Telegram notification are opt-in, off by default,
  toggled in settings. Nothing spawns and nothing calls out until switched on.
- **D8** — The cockpit can start new agents: pane creation, `agent.start`,
  configured presets.
- **D9** — Transcript is a second tab beside Terminal.
- **D10** — The terminal token is generated, shown and rotated on the settings
  page. **Known Risk** in CONTEXT.md applies and P1–P3 below discharge it.

## Planning resolutions

CONTEXT.md's *Deferred To Planning* list and its *Known Risk* section both
demand answers here. These are implementation decisions, cited downstream as
`per P<n>`; they never override a D-id.

- **P1 — the token never enters `Config`.** It lives in its own mode-0600 file
  beside the config (`~/.mdview/terminal.token`), following herdr-go's rule that
  secrets are never config fields. This is forced: `api_config`
  (`crates/mdview/src/server.rs:173-175`) serializes the entire `Config` as JSON
  on an unauthenticated route, so any token inside `Config` is one `curl
  /api/config` away regardless of what the settings HTML masks.
- **P2 — reveal once, then mask.** The settings page shows the token in full
  exactly once, in the response that generates or rotates it. Every later render
  shows only its last four characters. This is the named measure CONTEXT.md's
  Known Risk requires, chosen over loopback-restricted display because the peer
  address is not plumbed anywhere in this process — there is no `ConnectInfo`
  extractor and `crates/mdview/src/server.rs:80` serves without
  `into_make_service_with_connect_info`, so a loopback test would pass against
  the harness while proving nothing about production.
- **P3 — the D7 switches are written through a gated route, not
  `POST /api/config`.** `update_config` (`crates/mdview/src/server.rs:204`) is
  unauthenticated; leaving the supervisor switch on that form would let any LAN
  visitor make mdview spawn a process. The switches render on the settings page
  and post to a terminal-gated endpoint.
- **P4 — agent creation uses the same token as observe and reply.** A second
  credential for the create button buys nothing: whoever holds the token can
  already type any command into an existing agent.
- **P5 — rotating the token cuts live sessions immediately.** The in-memory
  session set is cleared on rotation.
- **P6 — no second allowlist.** herdr-go's `allowed_roots` does not come across.
  Inside a project, the path boundary enforces containment under that project's
  `root_path`; in the Unassigned group there is no containment claim to make, and
  the gate is what authorizes. One configuration surface, not two.

## Discovery

Inspected both codebases in full, then had the draft independently reviewed
against the repo. Findings that shape the plan:

- herdr-go is 16,168 lines of Rust across 40 files. Its dependency graph is
  shallow: `security`, `config`, `store`, `transcript`, `herdr/wire`,
  `herdr/fake`, `herdr/pane_scroller` have **zero** internal dependencies, and
  `herdr/socket.rs` has exactly one — a `#[cfg(windows)]` call to
  `crate::config::native_roaming_app_data()` at
  `herdr-go/src/herdr/socket.rs:35`. Those modules lift by copying. Everything
  under `herdr-go/src/web/` is welded to herdr-go's own `AppState`/`Config` and
  is rewritten, not copied.
- mdview already has the harness this needs: route-level axum tests through
  `router(state)` + `tower::ServiceExt::oneshot`, opening at
  `crates/mdview/src/server.rs:958`, with in-memory `SqliteStore` state. Auth
  tests have somewhere to live from day one.
- **`Config` has no injection seam on the handler path.**
  `crates/mdview-core/src/config.rs:114-122` hardwires `data_dir()` to
  `~/.mdview` with no override, and `settings_page_handler`
  (`crates/mdview/src/server.rs:182`) takes no `State` and calls `Config::load()`
  directly — so a route-level test of the settings page reads, and through
  `update_config` *writes*, the developer's real config file. The seam is
  prerequisite work, not a nicety.
- mdview has **no frontend build step anywhere** and vendors 3.5 MB of
  `mermaid.min.js` as `include_str!` (`crates/mdview/src/views.rs:1595`). That
  precedent decides how xterm.js eventually enters (~300 KB) and rules out
  porting herdr-go's 3,342-line Vite/TypeScript app.
- mdview has no per-project tab strip today — project navigation is a card grid
  at `crates/mdview/src/views.rs:82-115`. The "tabs" of D6/D9 are new UI.
- The store is only needed by notifications, never by the terminal
  (`herdr-go/src/store/mod.rs:1-3`: it "never stores terminal output or
  credentials"). The SQLite/outbox concern stays out of the early slices.
- `crates/mdview-desktop` is **excluded** from the workspace (`Cargo.toml:6`) yet
  depends on `mdview-core`. The declared evidence command cannot see a
  `mdview-core` API break reaching it.
- Evidence command: `cargo test --workspace` (`.bee/config.json`), green at
  planning time — 97 + 82 + 1 passed, 0 failed, ~135 s.

## Approach

See `approach.md` — recommended path, rejected alternatives, the risk map, and
the touch order.

## Shape

**Feature outcome.** Every registered mdview project has a Terminal tab and a
Transcript tab. The Terminal lists the coding agents running under that
project's root, shows each one's screen, sends typed replies and keys back, and
can start a new agent — all behind a token that gates those routes and nothing
else. Agents under no registered project stay visible in an Unassigned group.
Keeping herdr alive and notifying on status change ship off by default. herdr-go
is retired.

**Repo-reality basis.** The port is a copy, not a rewrite, because herdr-go's
core modules are dependency-free and carry ~2,000 lines of their own tests
(`herdr-go/src/herdr/socket.rs:567`, `herdr-go/src/herdr/fake.rs:757`,
`herdr-go/src/transcript/mod.rs:605`). The surface is a rewrite because every
handler takes `State<AppState>` of a type that ceases to exist. The browser leg
is a rewrite because mdview has no build step to receive TypeScript.

| Epic | Capability / Risk Area | Why It Exists | Slices | Proof Needed |
|---|---|---|---|---|
| E0 | A seam to test through | The settings and token work cannot be tested without a config path the harness can point somewhere safe. Today those tests would read and write the developer's real `~/.mdview/config.toml`. | S1 | A route-level settings test runs with `HOME` untouched and no write outside the test's own temp dir |
| E1 | Reach herdr | Nothing else can start until mdview can ask herdr what is running. The port carries its own tests. | S1 | `cargo test --workspace` green with the copied socket + fake tests running inside this workspace |
| E2 | The gate | D4's token, plus P1–P3 — the measure that keeps the token off `/api/config` and the D7 switches off the ungated form. The highest risk in the feature. | S1 | Every terminal route returns opaque 404 without a session; the token is absent from `GET /api/config`; a second settings render shows only its last four characters; `POST /api/config` cannot flip a D7 switch |
| E3 | See the agent | D2, D6 — the Terminal tab, the project-scoped pane list, the screen as text, the herdr-down state. The walking skeleton's visible half. | S1 | Against `FakeHerdr` through the route harness: a project's `/p/:id/_terminal` lists exactly the panes under its root and renders one pane's screen text; a silent socket renders a named remedy. Live-herdr confirmation is UAT at the gate, labelled as such |
| E4 | Talk back | D3 — free text and named keys reach the pane. | S2 | A posted reply reaches `pane.send_input`; send and submit stay distinct |
| E5 | Nothing lost | D5 — the Unassigned group, so no running agent disappears. | S2 | A pane under no registered root is listed and reachable, and its listing is behind the gate (see **Open at the gate**) |
| E6 | Fidelity | xterm.js vendored and the ANSI renderer, replacing E3's plain text. Split out because it carries none of S1's risk and is its largest chunk. | S3 | Colour and cursor-position output renders; the asset route serves the vendored bundle |
| E7 | Start agents | D8, P4 — pane and agent creation from presets. Starts processes from HTTP. | S3 | Unknown preset 400, unresolved anchor 409, `argv` never deserialized from the request |
| E8 | The other channel | D9 — the transcript tab, gap-free where the screen is not. | S4 | A tailed JSONL session renders as activity; the cursor survives a reload |
| E9 | Background duties | D7 — supervisor and notifier, both off until switched on. mdview's first process-spawning and first outbound call. | S5 | With both switches off, nothing spawns and nothing calls out; the switches are unreachable without the token (P3) |
| E10 | Retire herdr-go | D1 — the specs, the README, and every site still claiming mdview has no auth: `docs/specs/settings.md`, `docs/specs/web-interface.md:61,177,196`, `docs/specs/system-overview.md:83`, `crates/mdview-core/src/config.rs:64-67`, `crates/mdview/src/server.rs:69-77`. | S6 | Each named site states what is now true; `docs/specs/reading-map.md` carries the new area |

**Slice queue**

| Slice | Contents | Depends on |
|---|---|---|
| **S1** | E0 + E1 + E2 + E3 — the walking skeleton: a test seam, reach herdr, gate the routes, see one project's agents and one agent's screen as text | — |
| S2 | E4 + E5 — reply and keys; the Unassigned group | S1 |
| S3 | E6 + E7 — the ANSI renderer; agent and pane creation | S1, S2 |
| S4 | E8 — the transcript tab | S1 |
| S5 | E9 — supervisor and notifier behind their gated switches | S1 |
| S6 | E10 — retire herdr-go, correct every no-auth claim | S2–S5 |

S1 is a walking skeleton by the strict reading: end to end, real herdr client,
real screen content, real gate, no stubs. Plain text instead of xterm is lower
fidelity, not a stub — the screen a user reads is the screen herdr returned. The
gate ships **inside** S1 rather than after it, because a terminal surface that
exists ungated for even one merge is the exact failure D4 was written to prevent.

**Current slice to prepare: S1.**

## Test matrix

High-risk: probes per applicable dimension. Each cell's writer judges existing
coverage first (`.bee/expertise/tests.md`) and authors only the gap — the copied
herdr-go tests already pin several of these and must not be duplicated.

| Dimension | Probe |
|---|---|
| 1 — User types | No session cookie → every terminal route returns 404, not 401, and no route existence is leaked. A cookie minted before a rotation → refused (P5). |
| 2 — Input extremes | Empty reply body, a reply of control characters, a pane id containing `../`, a UTF-8 screen with wide CJK and emoji measured for column count. |
| 3 — Timing | Two screen polls overlapping; a reply posted while a poll is in flight; the token rotated mid-session. |
| 4 — Scale | 0 panes, 1 pane, 200 panes in one project; a screen at herdr's maximum line count. |
| 5 — State transitions | herdr up → down → up across polls; the tab rendered before herdr ever started; a pane that disappears between the list and the screen read. |
| 6 — Environment | Unix socket vs Windows named pipe; the socket file absent; the socket present but not accepting; a project root that is a symlink. Plus: `cargo test --workspace` writes nothing under the real `~/.mdview` (E0). |
| 7 — Error cascades | herdr returns a protocol error → the page shows a named state, never a raw error; a herdr timeout does not retry into a storm. |
| 8 — Authorization | Project A's page must never list or read a pane whose cwd is under project B; the boundary is checked on list, screen, input and keys, not only on the page route. `POST /api/config` cannot flip a D7 switch (P3). |
| 9 — Data integrity | Not applicable in S1 — the terminal writes no records and the registry is not extended (D5). Re-check at S5 when the notification outbox lands. |
| 10 — Integration | herdr protocol version mismatch fails loudly with a named state, never a silent empty screen. |
| 11 — Compliance | Screen and transcript text never reach mdview's logs. The token never appears in a log line, a URL, a masked settings render, or **`GET /api/config`** (P1). |
| 12 — Business logic | A pane exactly at the project root, one directory above it, and one below — boundary of the D2 containment rule. |

**Coverage the evidence command cannot reach:** `crates/mdview-desktop` is
excluded from the workspace (`Cargo.toml:6`) but depends on `mdview-core`. Any
cell changing `mdview-core`'s public surface builds it explicitly and says so in
its trace; `cargo test --workspace` will not catch that break.

## Open at the gate

One conflict between two locked decisions that planning must not resolve alone:

**D5 places the Unassigned group on the project list page; D4 leaves that page
ungated.** As written, an unauthenticated visitor would see every agent running
outside every registered project, cwd included. This plan's default is to put
the group's *contents* behind the gate — the home page shows the group exists,
the agents inside it need the token. That honors D5's purpose (nothing
disappears) at the cost of its letter (the list is on the home page). Overrule
at the gate if the literal reading was intended.

## Out of scope

- herdr-go's self-update (`herdr-go/src/update/`) — a standalone CLI verb that
  never touches the terminal path; mdview has its own release flow.
- herdr-go's OS service install and `herdr-go service <verb>` — mdview already
  has its own daemon lifecycle (`docs/specs/daemon.md`); reconciling the two is
  separate work (deferred in CONTEXT.md).
- The Cloudflare Access JWT fallback — D10 and P1–P2 settle the token mechanism.
- Moving the screen poll onto `/ws` — deferred, revisit once the surface is real
  (see `approach.md`).
- Mobile-specific layout work carried over from herdr-go's mobile-first UI.
