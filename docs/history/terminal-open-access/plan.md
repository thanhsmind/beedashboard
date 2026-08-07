---
artifact_contract: bee-plan/v1
mode: high-risk
# approved_gate2: <unset>
---

# Plan: Terminal Open Access

Mode: `high-risk` — 5 risk flags: auth, audit-security, public-contracts, proof-weakening, multi-domain
Why this is the least workflow that protects the work: this removes an authentication boundary from a surface that runs shells and starts agents. The danger is not the removal, which the owner decided; it is that a 1001-line module and 126 call sites come out in one sweep, and the guard that must NOT come out — containment — lives in the same handlers as the guards that must.

## Requirements (from CONTEXT.md)

- **D1** No token, no session, no login route, no cookie. The `terminal_auth` module goes.
- **D2** `terminal.enabled` is the only gate; off means mdview's ordinary 404, not a disguised one.
- **D3** The two background switches, the notify destination and the notify credential become ordinary settings.
- **D4** The notify credential stays a write-only, owner-only file.
- **D5** The method gate goes with the disguise it served.
- **D6** **Containment survives untouched** — a pane outside the project root is still refused, project scoping still holds, the fail-closed empty pane list still fails closed.
- **D7** Every retired auth test is named with its reason. Silent deletion is forbidden.
- **D8** The four living specs stop describing an authentication that no longer exists.
- **D9** The Unassigned group survives behind its own switch, **default off** — it reaches every herdr pane on the host and has no containment check of its own.
- **D10** The switch endpoint accepts a **JSON body only**, so a cross-site form submission cannot flip it using the owner's own Access cookie.
- **D11** Every terminal route is mounted with its **explicit method**, in the same change that removes the method gate.
- **D12** Disabled means the ordinary not-found page for page routes, and a reasoned JSON 404 for the polled data routes.

## Discovery

`crates/mdview/src/terminal_auth.rs` is 1001 lines. `crates/mdview/src/server.rs` carries 126 references to `terminal_auth`, `AuthSession` or `MethodGate`, and 490 of its lines mention a session or a token. The terminal route family is already gated on `terminal_family_enabled`, so D2 needs no new mechanism — it needs the existing check to become the only one and to answer with the ordinary not-found path.

The exposure this rests on is external and was verified with the owner, not in the code: Cloudflare Access fronts the tunnel, and port 7700 is blocked from LAN and tailnet at another layer. The safety of everything below is that statement. It belongs in the spec, not only in a decision log, because the next person to open port 7700 needs to meet it.

## Approach

**Strip the call sites first, delete the module second.** Removing the extractors from the handlers is reversible and testable on its own; deleting a 1001-line module is neither. Landing them as separate cells means the suite is green in between, and a regression is attributable to one of the two.

*Rejected — one sweeping deletion.* It is fewer steps and it is exactly how D6 gets lost: the containment checks sit inside the same handlers as the auth extractors, and a diff that large stops being read line by line.

*Rejected — keeping the module but making it always-allow.* Dead authentication that always says yes is worse than none: it reads like protection in every future review of this file.

**Risk map**

| Component | Risk | Proof needed |
|---|---|---|
| The Unassigned group widening past what was agreed (D9) | **HIGH** | Its switch is absent or off in every fixture but the one that tests it on; with it off, its routes are not found, proven per route. The group reaches panes outside every project and has no containment check — the session gate was its authorization. |
| A cross-site page flipping the switches (D10) | **HIGH** | A form-encoded POST to the switch endpoint is refused; only a JSON body is accepted. Without this, any site the owner visits while signed in to Access can start a supervised process on their machine. |
| A GET flipping the switches through the query string (D11) | **HIGH** | A GET carrying switch values in its query changes no switch — `Form` reads the query on GET in axum 0.7.9, so the method gate was load-bearing, not decorative. |
| Containment lost inside the auth deletion (D6) | **HIGH** | The pane-outside-project-root test, the project-scoping test and the fail-closed empty-list test are named, run, and green after every cell — assertions and fixtures unmodified, only their login scaffolding removed. |
| A retired test taking a live rule with it (D7) | **HIGH** | Every deletion named with its reason in the cell's outcome; a reviewer checks the list against the retired tests before the feature closes. |
| Specs left describing auth that is gone (D8) | MEDIUM | Each of the four specs read after the change; no sentence claims a token, a session or a login. |
| The disabled state regressing to the bare 404 | LOW | With the terminal off, the route answers with mdview's ordinary not-found page, content-type and body — not the typeless empty response that made browsers download a file. |

## Shape

| Slice | Contents | Depends on |
|---|---|---|
| **S1** (current) | Strip the auth and method extractors from every terminal route and **re-mount each with its explicit method in the same cell** (D11). `terminal_family_enabled` becomes the only gate, answering with the ordinary not-found page for page routes and a reasoned JSON 404 for the polled data routes (D12). The switch endpoint takes a JSON body only (D10). Containment untouched (D6). The keep-list below is written before any test is retired. | — |
| **S2** | Delete the `terminal_auth` module, the login and rotation routes, and the settings page's token controls. | S1 |
| **S3** | Ungate the two background switches, the notify destination and the notify credential (D3), keeping the credential's owner-only write-only file (D4). | S2 |
| **S4** | Put the Unassigned group behind its own switch, default off (D9). | S3 |
| **S5** | Re-sync `agent-terminal.md`, `settings.md`, `web-interface.md`, `bee-cockpit.md`; fix `assets/app.js`'s now-false session-expired copy; clear the dangling `terminal_auth` doc references in `crates/mdview-core/src/config.rs` and `views.rs`; and record the firewall assumption this surface now depends on. | S4 |

## Test matrix

| Dimension | Probe | Status |
|---|---|---|
| Containment | A pane outside the project root; a pane in another project; a boundary that fails to construct | **Already pinned — assertions stay, login scaffolding goes** |
| Disabled state | Terminal off answers with the ordinary not-found page, not a typeless empty response | New |
| Enabled state | Every terminal route answers with no cookie and no token present | New |
| Untrusted content | A pane title and a workspace path containing markup render as text | Already pinned |
| Secret at rest | The notify credential is still written owner-only and never read back into the page | Already pinned |
| Retired proof | The named list of retired tests matches what the diff actually removed | New, checked at close |

### The keep-list — written before anything is retired

A by-name retirement ledger without a keep-list retires live rules. These tests carry a session or a token in their NAME but their subject is something else, and every one of them survives, re-expressed without the login scaffolding:

- the six `*_disabled_*_even_with_a_valid_session` tests (`server.rs:7086, 7633, 8596, 9573, 9613, 10295`) — the only coverage that a disabled switch actually refuses;
- `gated_switch_route_starts_and_stops_the_live_background_tasks` (`:6161`) — subject is the background tasks;
- `transcript_poller_distinguishes_session_expired_from_a_transient_error` (`:8192`) — subject is the client's named-state behaviour;
- `post_api_config_with_terminal_fields_leaves_every_switch_unchanged` (`:6058`) and its notify twin (`:6359`) — subject is field separation;
- `unauthenticated_home_page_shows_unassigned_presence_only_and_leaks_nothing` (`:9234`) — subject is presence-only rendering;
- `settings_and_terminal_config_switch_stay_reachable_while_terminal_disabled` (`:7143`) — subject is being able to turn it back on;
- `api_terminal_config_wrong_method_is_byte_identical_to_unrouted` (`:6769`) — re-expressed as "a GET carrying switch values in the query changes no switch" (D11), never retired.

Beyond containment, these non-auth guards must stay green: pane ids never trusted from the URL; the two groups partitioning without overlap; argv operator-authored only, unknown preset refused before herdr; the thousand-key input bound; `submit` defaulting to staged-not-sent; pane output escaped before markup; a named remedy instead of a raw error or path; typed text and named keys never logged; the notify credential write-only at rest.

Every cell caps through `bee finish`, which runs `cargo test --workspace`.

## Out of scope

- Verifying Cloudflare Access JWTs, or any replacement authentication. The owner chose none.
- Changing what the terminal does, its pane model, or its transcript.
- Re-binding the daemon. It stays on its configured host and port.
