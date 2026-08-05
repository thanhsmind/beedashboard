# Approach: Agent Terminal

## Recommended path

Copy herdr-go's already-pure modules into `mdview-core` with their own tests
intact, rather than rewriting them; put the axum surface and the HTML in
`crates/mdview` following the `_bee` page family exactly (per D2). The herdr
client, the path boundary, the transcript reader, the store, the notifier, the
watcher and the supervisor are all zero- or one-edge modules in herdr-go's own
dependency graph and carry roughly 2,000 lines of passing tests with them —
copying preserves that proof, a rewrite discards it. Everything welded to
herdr-go's `AppState` and `Config` (the handlers, the doctor checks) is rewritten
against mdview's own state and config, because that weld is exactly what
absorption dissolves.

The browser leg is rewritten, not copied: herdr-go's 3,342-line Vite/TypeScript
app cannot enter a workspace with no frontend build step. `xterm.js` is vendored
as a compiled-in `const` the way `mermaid.min.js` already is (3.5 MB precedent
versus ~300 KB for xterm), and the client logic joins `app.js` as vanilla JS.
Only `terminal.ts`'s poll-and-render core (677 lines) and the pane switcher
(416 lines) have to survive the translation; `login.ts`, `main.ts` and the Vite
router do not.

Order follows risk, not layers. Slice 1 is a walking skeleton that already
carries the token gate (per D4), because the dangerous capability and the
surface that exposes it must never exist apart — not even on a branch.

## Rejected alternatives

- **Depend on the `herdr_go` crate instead of copying it** — herdr-go is
  `lib+bin`, so this compiles. Rejected: D1 retires herdr-go as a product, and a
  path/git dependency on an unreleased sibling repo keeps it alive as a build
  input while adding a release coupling mdview does not want.
- **Rewrite the herdr client from the protocol description** — rejected: throws
  away ~475 lines of passing socket tests and the `FakeHerdr` seam every web
  test in herdr-go depends on, for no gain.
- **Introduce a Vite build step for the terminal UI** — rejected: it would be
  the workspace's first frontend build, changing how every contributor builds
  mdview, in exchange for reusing TypeScript that mostly implements routing and
  login mdview already has.
- **Move the screen poll onto the existing `/ws` broadcast** — deferred, not
  rejected. herdr's socket is request/response only, so a push transport changes
  only the browser leg. Keeping HTTP polling in slice 1 matches herdr-go's proven
  behavior and leaves the upgrade cheap. Revisit once the surface is real.
- **Ship the terminal read-only first and add reply later** — rejected as the
  slice-1 boundary: D3 makes reply the point of the feature, and splitting it out
  does not reduce the auth work, which is what actually carries the risk.
- **Port `src/update/` (self-update)** — out of scope. It is a standalone CLI
  verb that never touches the terminal path, and mdview has its own release flow.

## Risk map

| Component | Risk | Reason | Proof needed |
|---|---|---|---|
| Token gate on terminal routes (D4) | **HIGH** | First auth in a codebase that documents itself as having none; an unauthenticated bypass hands a LAN keystrokes into a live agent | Route-level tests through `router(state)` + `oneshot` (the harness opening at `crates/mdview/src/server.rs:958`) proving every terminal route returns opaque 404 without a session, on list, detail, screen, input and keys alike |
| Token issuance on the settings page (D10) | **HIGH** | Settings is unauthenticated by D4; showing the token there voids the gate. `api_config` (`crates/mdview/src/server.rs:173-175`) also dumps the whole `Config` as JSON unauthenticated, so storing the token in `Config` leaks it whatever the HTML masks | Per P1–P2: the token is absent from `GET /api/config`; the first render after generation carries it in full; every later render carries only its last four characters |
| The D7 switches on `POST /api/config` | **HIGH** | `update_config` (`crates/mdview/src/server.rs:204`) is unauthenticated — a supervisor switch there lets any LAN visitor make mdview spawn a process | Per P3: a `POST /api/config` carrying a supervisor field leaves the switch unchanged |
| No config seam on the handler path | **MEDIUM** | `data_dir()` is hardwired to `~/.mdview` (`crates/mdview-core/src/config.rs:114-122`) and `settings_page_handler` (`:182`) takes no `State` — settings tests would read and write the developer's real config | `cargo test --workspace` writes nothing under the real `~/.mdview` |
| Agent creation (D8) | **HIGH** | Starts a process from an HTTP request | Port herdr-go's own refusals: unknown preset → 400, unresolved anchor → 409, `argv` never accepted from the client (`src/web/create.rs:26-41`) |
| Project-root scoping of panes (D2/D5) | **MEDIUM** | A pane leaking into the wrong project is a cross-project information leak | Path-boundary tests over symlinks and `..`, reusing `security/paths.rs`'s 7-step gate |
| herdr client port | **LOW** | Pure module, ~475 lines of tests travel with it | `cargo test --workspace` green after the copy |
| Vendored xterm.js | **LOW** | Precedent exists (`mermaid.min.js`, 3.5 MB, `include_str!`) | Binary builds; asset route serves it |
| Windows named pipe | **MEDIUM** | mdview carries a cross-platform flag; the socket path has a `#[cfg(windows)]` edge into herdr-go's config (`src/herdr/socket.rs:35`) | The edge is replaced by an injected path; compile check on the Windows target |
| Supervisor spawning `herdr` (D7) | **MEDIUM** | mdview would start processes it never started before | A test proving nothing spawns while the switch is off |
| Telegram notifier (D7) | **LOW** | Off by default, `Notifier` trait already pluggable (`src/notify/mod.rs:32-35`) | A test proving no outbound call while the switch is off |

## Files and order

Likely touch order, leaves first:

1. `crates/mdview-core/src/config.rs` — the injectable data dir (E0), then the
   terminal section and the D7 switch fields. The token is **not** a field (P1).
2. `crates/mdview-core/src/herdr/` — new module tree copied from herdr-go
   (`mod.rs`, `socket.rs`, `wire.rs`, `fake.rs`), the `#[cfg(windows)]` config
   edge replaced by an injected path. `pane_scroller.rs` waits for S3.
3. `crates/mdview-core/src/paths_boundary.rs` — from
   `herdr-go/src/security/paths.rs`.
4. `crates/mdview/src/terminal_auth.rs` — new: token file, session set,
   extractor, opaque 404, reveal-once state (P1, P2, P5).
5. `crates/mdview/src/views.rs` — the terminal page, the tab strip, the settings
   additions.
6. `crates/mdview/src/server.rs` — the route family, its handlers, and the gated
   switch endpoint (P3).
7. `crates/mdview/assets/app.js` — the poll client.
8. Later slices: `crates/mdview/assets/xterm*` and the ANSI renderer;
   `transcript.rs`; `notify/`, `store/`, `watcher.rs`, `supervisor.rs`.
9. `docs/specs/*`, `README.md` — the retirement and every corrected no-auth claim.

## Questions still open

All three of CONTEXT.md's deferred questions are answered in `plan.md`
("Planning resolutions"): no second allowlist (P6), agent creation shares the
observe/reply token (P4), rotation cuts live sessions immediately (P5).

What remains is not a planning question but a conflict between two locked
decisions — D5 places the Unassigned group on the ungated project list page —
raised for the user under `plan.md` § **Open at the gate**.
