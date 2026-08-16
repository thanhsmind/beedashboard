# Review P1 Fixes — Context

**Feature slug:** review-p1-fixes
**Date:** 2026-08-16
**Shaping session:** complete
**Scope:** Deep
**Domain types:** CALL | RUN

## Feature Boundary

Fix the six P1 findings from review `review-2026-08-16-all-unreviewed` and nothing
else: the four security holes that turn a click or a viewed markdown file into RCE
on the developer's machine, the red CI gate, and the one behavior-change cell shipped
without reachable evidence. P2/P3 findings are out of scope (they go to backlog).

## Locked Decisions

Each remedy was named in a verified finding and the user approved fixing all P1s.
Cited, never reinterpreted.

| ID | Decision | Rationale |
|----|----------|-----------|
| D1 | Add one Tower middleware layer on the router that rejects any request whose `Host` header is not in {`127.0.0.1`, `localhost`, `[::1]`, the configured hostname} with HTTP 421. | Closes DNS-rebinding (finding: no Host/Origin check) and both CSRF findings (`/api/config`, register/unregister/refresh) in one place — a cross-origin/rebound page cannot forge a valid loopback Host. |
| D2 | Escape the title inside `layout()` itself (`esc(title)`), so no caller can reintroduce the injection. | Stored+reflected XSS via `<title>` (views.rs:23); fixing at the single sink covers all ~12 unescaped callers. |
| D3 | Remove the generic `data-*` attribute prefix from the markdown sanitizer; allow only the specific `data-*` names the renderer legitimately emits. Additionally, validate `data-term-base` in app.js as a same-origin `/p/…` shape before using it as a fetch prefix. | Open `data-*` + `innerHTML` sink (render.rs:461, app.js:1072) lets a markdown file point the poller at an attacker origin. Defense at both the sanitizer and the JS sink. |
| D4 | Reject a `:feature` URL segment containing `/`, `\`, or any `..`/`.` path component before it is joined to `.bee/cells/archive/`, matching the `starts_with(root)` containment the rest of the code enforces. | Path traversal out of project root (bee.rs:1931) via percent-decoded segment. |
| D5 | Make CI green on HEAD: `cargo fmt --all`, fix the 10 `cargo clippy -D warnings` errors, and widen `commands.test` to run the full CI triple (fmt check + clippy + test) so a cell cap cannot pass a gate CI would fail. | Every cell's "green" measured one third of the declared gate; the branch would fail CI on merge. |
| D6 | Re-verify `homepage-terminal-refresh-1`: land a node-free unit test over the pure `shouldReload` predicate (the changed behavior), or, if a harness is genuinely out of reach this slice, record the JS-only gap explicitly the way `home-terminal-header-2` did. | A behavior-change cell whose only evidence is a Rust run that cannot reach the changed `app.js`. |

### Agent's Discretion

- Exact middleware placement and 421 body; whether to read the configured hostname
  from existing config.
- The precise allowlist of legitimate `data-*` names emitted by the renderer
  (`data-sourcepos` and any the code actually produces) — grep the emitter, do not guess.
- Whether D6's JS harness is a tiny standalone runner or a `#[test]` shelling a
  node-free evaluator; if neither is cheap, the recorded-gap fallback is acceptable.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| loopback Host | a `Host` header naming only the local machine — the set D1 allows |
| same-origin term base | a `data-term-base` that resolves under this daemon's own `/p/<project>/` path |

## Existing Code Context

### Integration Points

- `crates/waggledance/src/server.rs` — router (~407), `update_config` (:1082),
  register/unregister/refresh (:1139–1224); D1 middleware, D5 clippy.
- `crates/waggledance/src/views.rs` — `layout()` (:16), the `esc` helper (:5960);
  D2, D5 clippy/fmt.
- `crates/waggledance-core/src/render.rs` — sanitizer (:457–465); D3.
- `crates/waggledance-core/src/bee.rs` — `read_archived_cells` (:1931); D4, D5 clippy.
- `crates/waggledance/assets/app.js` — poller `data-term-base` use (:1033, :1072);
  D3 JS-side; D6 `shouldReload` (:819).

## Canonical References

- Review `review-2026-08-16-all-unreviewed` findings (`.bee/reviews/`) — the source
  of every decision above.
- `docs/specs/agent-terminal.md:333–352` — the containment guarantees D1 restores.

## Outstanding Questions

### Deferred To Planning

- [ ] The 10 clippy errors' exact fixes (D5) — read each at its file:line; several are
      `too_many_arguments` (bee.rs:1487) and `sort_by_key` lints.
- [ ] Whether D3's app.js validation belongs in `screenUrl` or at each `data-term-base`
      read — decide from how many call sites read the attribute.

## Deferred Ideas

- CSP header, WebSocket origin check, unbounded `.bee` read caps, asset caching,
  `views.rs` split, `bee_hub_card` arg struct, app.js JS harness beyond `shouldReload`,
  the three-poller `inFlight` unification, the Unassigned inline-script fold — all P2/P3,
  filed to backlog, not this feature.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. The six decisions map one
to one onto the six P1 findings; the fix for each caps only against a test that proves it.
