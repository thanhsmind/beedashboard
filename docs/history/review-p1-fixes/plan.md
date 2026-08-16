# Review P1 Fixes — Plan

**Lane:** high-risk (flags: audit-security, public-contracts). All decisions locked in
CONTEXT.md; each cell caps only against a test proving its fix.

## Shape: six cells, security first

Ordered by leverage. Cells 1–4 are the security boundary; 5–6 are verification hygiene.
Cells are largely file-disjoint so most can run in parallel; cell 5 (CI green) runs last
because fmt/clippy touch every file the others just edited.

| Cell | Fix (decision) | Primary file | Proof |
|------|----------------|--------------|-------|
| review-p1-fixes-1 | Host-header middleware, 421 on non-loopback (D1) | server.rs | tests: loopback Host passes; a foreign Host on `/api/config`, `/api/terminal-config`, register/unregister and a GET all get 421; missing Host handled |
| review-p1-fixes-2 | `esc(title)` in `layout()` (D2) | views.rs | test: a `</title><script>` payload through search `?q=` and through a doc H1 renders escaped in `<title>`; existing pages still render |
| review-p1-fixes-3 | Drop generic `data-*` from sanitizer + same-origin `data-term-base` check (D3) | render.rs, app.js | test: a markdown `<pre data-term-base=https://evil>` loses the attribute through `sanitize()`; legitimate `data-*` the renderer emits survive |
| review-p1-fixes-4 | Reject traversal in `:feature` segment (D4) | bee.rs | test: `read_archived_cells` with `..`/separator-bearing feature returns empty and reads nothing outside root; a normal feature still reads its cells |
| review-p1-fixes-5 | CI green: fmt, 10 clippy errors, widen `commands.test` (D5) | (all) + .bee/config | proof: `cargo fmt --all --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green — quoted fresh |
| review-p1-fixes-6 | Re-verify `homepage-terminal-refresh-1` (D6) | app.js / test | proof: a node-free assertion over `shouldReload` for the `.term-screen` / Kanban / Projects cases, or a recorded-gap note if unreachable |

## Ordering & dependencies

- 1, 2, 3, 4 independent (disjoint files bar app.js shared by 3 and 6) → run in parallel.
- 6 touches app.js `shouldReload`; 3 touches app.js `data-term-base`. Different functions,
  but reserve app.js so they serialize if both land there.
- 5 runs after 1–4 and 6 cap, because fmt/clippy must see their final code, and D5 widens
  the test command every subsequent cap uses.

## Verify scoping

Declared gate after D5 = the CI triple (fmt check + clippy + test). Until D5 caps, cells
run the current `cargo test --workspace`; cell 5's own cap is the first to run the widened
command and must be green on all three.

## Rollback

Each cell is one commit on the feature branch; the branch merges as a unit. A single fix
that regresses is revertable by its commit without touching the others. The middleware
(D1) is the only change that alters request handling for every route — its test covers a
loopback request passing unchanged, so a mistake there fails the suite rather than silently
locking the daemon out.
