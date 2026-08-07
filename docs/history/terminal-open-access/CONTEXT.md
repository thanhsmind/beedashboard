# Terminal Open Access — Context

**Feature slug:** terminal-open-access
**Date:** 2026-08-07
**Shaping session:** complete
**Scope:** Standard
**Domain types:** CALL | ORGANIZE

## Feature Boundary

The agent terminal stops authenticating callers. Its access token, session, login
route and opaque-404 disguise are removed, and its routes become ordinary mdview
routes reachable by anyone who can reach the daemon. The `terminal.enabled`
switch remains the only thing that turns the surface on or off. Nothing about
what the terminal *does* changes, and no guard that is not authentication is
touched.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | The terminal has **no authentication of its own**: no access token, no session, no login route, no cookie, no reveal-once secret. The whole `terminal_auth` module goes. | The owner's call, made with the exposure stated. |
| D2 | `terminal.enabled` is the only gate. Off, the terminal routes are **not found in the ordinary way** — mdview's normal 404, not a disguised one. | The opaque 404 existed to hide the terminal from an unauthenticated prober. With no authentication there is nobody to hide from, and a bare typeless 404 is what made browsers download a file instead of showing a page. |
| D3 | The two background switches (keep herdr running, notify on status change), the notify destination and the notify credential lose their session requirement and become ordinary settings, like every other field on that page. | They were session-gated only because the terminal was; that carve-out has no basis once D1 lands. |
| D4 | The notify credential stays a write-only, owner-only file that is never read back into the page. | Protecting a secret at rest is unrelated to route authentication and stays exactly as it is. |
| D5 | The method gate goes with the disguise it served. Terminal routes answer with ordinary method semantics. | Its only purpose was closing a 404/405 oracle that revealed the routes existed. |
| D6 | **Containment survives untouched.** A pane outside the project root is still refused; the path boundary, the project scoping, and the fail-closed empty pane list are not authentication and must keep working. Their **assertions and fixtures stay unmodified**; only the session scaffolding they currently set up is removed — every one of them logs in today, so "unmodified" in the literal sense is unsatisfiable. | This is the decision most likely to be lost in a large deletion, and losing it would let any caller drive a terminal in a directory the project never owned. |
| D7 | Every test that asserted authentication is **retired by name with its reason** in the work record. Silent deletion is forbidden. A test whose subject was the disguise is retired; a test whose subject was containment, method correctness for its own sake, or any non-auth guard is kept and re-expressed. | A large auth removal is exactly where a containment test disappears unnoticed inside the diff. |
| D8 | Every living spec that describes the terminal's authentication is corrected in the same work — `agent-terminal.md`, `settings.md`, `web-interface.md`, `bee-cockpit.md`. Their no-auth statements revert to plain, and the D4 carve-out language is removed rather than left dangling. | This repo has already shipped two specs asserting the opposite of the code; that is not repeated. |

| D9 | The **Unassigned group survives behind its own switch, default off**. It reaches panes outside every registered project and has no containment check of its own — the session gate was its authorization (`server.rs:1503-1505`). With authentication gone it needs a deliberate act to turn on, and turning it on means accepting that every herdr pane on the host is readable and writable through the page. | Found by the advisor, not by the plan: D6 does not cover this group because there is nothing there to preserve. Off by default is what keeps the owner's decision from silently widening past what they agreed to. |
| D10 | The switch endpoint accepts a **JSON body only**, never a form. | A form-encoded POST is a CORS simple request: no preflight, and mdview has no CORS layer. Today `AuthSession` plus `SameSite=Strict` makes a cross-site submission inert; after D1 nothing would. A JSON body forces a preflight, so a page the owner happens to be visiting cannot flip the background switches, redirect notifications, or overwrite the credential using the owner's own Access cookie. This costs no dependency and no login. |
| D11 | Every terminal route is **mounted with its explicit method**, not `any(...)`, in the same change that removes the method gate. | Not disguise — correctness. In axum 0.7.9 `Form` reads the query string when the method is GET (`form.rs:85`, `raw_form.rs:41-42`), so with the gate gone and the route still mounted `any`, a plain navigation or an `<img src>` to `GET /api/terminal-config?enabled=on&supervisor_enabled=on` would flip the switches. |
| D12 | The disabled state answers with mdview's **ordinary not-found page** for page routes, and a **404 with a JSON body naming the reason** for the data routes the client polls. "Byte-identical to an unrouted path" is struck: the router has no fallback, so an unrouted path is the typeless empty response that made a browser download a file. | The client's pollers must not be handed an HTML page, and the browser must not be handed nothing. |

### Agent's Discretion

How the removal is sliced, and whether `terminal_auth` is deleted outright or
reduced to whatever non-auth helpers other code still needs — provided nothing
in D6 regresses.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Containment | The rule that a pane must live under a registered project's root. Not authentication; survives this feature. |
| Disguise | The opaque 404 and the method gate together — machinery whose only job was making the terminal routes indistinguishable from routes that do not exist. |

## Specific Ideas And References

- The owner runs Cloudflare Access in front of `mdview.gogl.be` (team domain
  `goglbe.cloudflareaccess.com`), and confirmed port 7700 is blocked from the
  LAN and the tailnet at another layer, leaving the tunnel as the only route in.
  **This feature's safety rests entirely on that outer layer.** If it is ever
  removed, the terminal becomes unauthenticated remote code execution for anyone
  who can reach the port, and this decision must be revisited.

## Existing Code Context

### Integration Points

- `crates/mdview/src/terminal_auth.rs` (1001 lines) — the module being removed.
- `crates/mdview/src/server.rs` — 126 references to `terminal_auth`,
  `AuthSession` and `MethodGate` across the terminal route family, plus the
  settings token UI and the login and rotation routes.
- `crates/mdview/src/views.rs` — the settings page's token controls.

### Established Patterns

- The terminal route family is already gated on `terminal_family_enabled`; that
  check stays and becomes the only one.

## Canonical References

- `docs/specs/agent-terminal.md` — the living spec for this surface.
- `docs/specs/settings.md`, `docs/specs/web-interface.md`,
  `docs/specs/bee-cockpit.md` — carry no-auth statements scoped to the carve-out
  this feature removes.

## Outstanding Questions

### Deferred To Planning

- [ ] Whether any non-auth helper in `terminal_auth.rs` is used elsewhere and
      must be rehomed rather than deleted.
- [ ] Whether the settings page keeps a terminal section at all once the token
      controls are gone.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. D6 and D7 are the
two most likely to be lost in a large deletion and are the reviewer's first
checks.
