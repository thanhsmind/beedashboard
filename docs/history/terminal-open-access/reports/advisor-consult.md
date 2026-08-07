PROCEED WITH CONDITIONS — high-risk advisor consult, terminal-open-access, 2026-08-07.

Scope note: the owner's decision to remove the terminal's authentication was not relitigated. The consult covered what can still go wrong.

Fifteen non-auth guards enumerated in the terminal handlers, each with its pinning test: the per-project path boundary; creation-destination containment; fail-closed behaviour on an unconstructable boundary; pane ids never trusted from the URL; the two groups partitioning without overlap; argv operator-authored only; the thousand-key input bound; submit defaulting to staged-not-sent; pane output escaped before markup; a named remedy instead of a raw error; typed text and keys never logged; the notify credential write-only at rest; terminal fields not settable through the general config route; the home page revealing presence only; settings staying reachable while the terminal is off.

Three findings that changed the work:

1. THE UNASSIGNED GROUP HAS NO CONTAINMENT CHECK AT ALL. `server.rs:1503-1505` says so outright — the session gate is what authorizes it, and `verify_pane_is_unassigned` confirms a pane is under no registered project root, the opposite of containment. Removing authentication makes every herdr pane on the host readable and writable: unrelated repos, root shells, other agents. Owner's resolution: keep the group behind its own switch, default off (D9).

2. THE SWITCH ENDPOINT BECOMES CROSS-SITE WRITABLE. `update_terminal_config` takes a form body — a CORS simple request, no preflight, and there is no CORS layer. Today AuthSession plus SameSite=Strict makes a cross-site submission inert; afterwards, any site the owner visits while signed in to Cloudflare Access could flip `supervisor_enabled` (mdview spawns a process), redirect the notify destination, or overwrite the credential, using the owner's own Access cookie. Owner's resolution: JSON body only, which forces a preflight (D10).

3. REMOVING THE METHOD GATE IS A CORRECTNESS CHANGE, NOT DISGUISE. In axum 0.7.9 `Form` reads the query string when the method is GET (`form.rs:85`, `raw_form.rs:41-42`). With the gate gone and routes still mounted `any(...)`, `GET /api/terminal-config?enabled=on&supervisor_enabled=on` flips the switches — triggerable by a plain navigation or an `<img src>`. Resolution: every route re-mounted with its explicit method in the same change (D11).

Further conditions, all folded into the plan:

4. The enabled-check itself is written in terms of the module being deleted — twelve sites return `terminal_auth::opaque_404()`. A removal that deletes lines mentioning `terminal_auth::` leaves `if !terminal_family_enabled(&st) { }` — the only surviving gate, silently disarmed, still compiling.

5. D6's "tests green and unmodified" is unsatisfiable as written: every containment test logs in first. Restated as assertions and fixtures unmodified, login scaffolding removed.

6. A by-name retirement ledger without a keep-list retires live rules. Twelve tests carry "session" or "token" in their name while their subject is something else — the six disabled-state tests are the only coverage that a disabled switch actually refuses. The keep-list is written into the plan before anything is retired.

7. The disabled answer must be named per route family. The router has no fallback, so "byte-identical to an unrouted path" means the typeless empty response that made a browser download a file. Page routes answer with the ordinary not-found page; polled data routes answer 404 with a reasoned JSON body. `assets/app.js`'s session-expired copy becomes false and is added to a slice.

Bookkeeping: eight dangling `terminal_auth` doc references in `crates/mdview-core/src/config.rs` and one in `views.rs`.

Disposition: all conditions written into `docs/history/terminal-open-access/plan.md` and CONTEXT.md (D9-D12) before the gate was recorded. Consult identity: bee-review subagent, review tier, read-only; this repo configures no `models.claude.advisor`, and the gap is named here rather than skipped.
