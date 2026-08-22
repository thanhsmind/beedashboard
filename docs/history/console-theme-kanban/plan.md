---
artifact_contract: bee-plan/v1
mode: standard
# approved_gate2: <unset until approval>
---

# Plan: Console theme + homepage kanban rebuild

Mode: `standard` — 3 risk flags: multi-domain, public-contracts,
covered-contract-change.
Why this is the least workflow that protects the work: the token contract is a
published surface and several tests pin exact CSS literals that this change
must move; a phase plan keeps the theme swap provable on its own before any
board markup moves under it.

## Requirements (from CONTEXT.md)

- **D1** (`b27a73c6`) — The console look replaces Atelier everywhere. One
  shipped theme; the theme adapter is swapped, the component layer
  (`contract.css`, `components.css`, `editorial.css`) stays untouched, and the
  board's page-local palette override is deleted so the board inherits the one
  theme.
- **D2** — Cards render only elements backed by real data in bee's store. No
  placeholder, zero, or em-dash stands in for a missing source.
- **D3** — The phone screen is responsive CSS over the same markup. No second
  route, no separate mobile surface.

Inherited boundaries: the cockpit is read-only, so the screenshots' `Merge PR`
button renders as a *state* and the mobile `+` FAB is dropped;
`docs/specs/bee-cockpit.md` is re-synced as part of this feature; the tests
that assert exact CSS literals move in lockstep with the CSS.

## Discovery

Three read-only sweeps mapped the theme layer, the board render path, and the
store's real data surface.

- **The theme is one file.** `crates/waggledance/assets/atelier/atelier.css`
  (224 lines) is the sole Tier-1→Tier-2/3 adapter; `contract.css` (405),
  `components.css` (572) and `editorial.css` (128) read only the contract and
  never a primitive. The bundle is assembled by `include_str!` concatenation at
  `crates/waggledance/src/server.rs:6324` and served from `server.rs:1629`.
  D1's "swap the adapter" is therefore literally one file plus the concat list.
- **Scheme switching is attribute-driven, not media-query-driven.** A no-flash
  head script (`views.rs:25-47`) resolves `localStorage` + `matchMedia` into
  `data-scheme`, and
  `dark_scheme_rules_present_with_no_os_media_query_to_override_them`
  (`server.rs:21643`) asserts the shipped bundle contains **no**
  `prefers-color-scheme` at all. The console theme must express dark and light
  through `data-scheme`, never through an OS media query.
- **The board's palette override exists and is pinned by a test.**
  `bee_hub_style()` (`views.rs:1824-2203`) is a body-level `<style>` block
  whose lines 1837-1916 redefine `--color-*` for `.bee-hub-theme`;
  `feature_hub_theme_tokens_render_for_both_light_and_dark` (`server.rs:6335`)
  asserts `--color-bg: #FAF9F5;` and five more literals. D1 deletes that block,
  so this test is rewritten, not merely edited.
- **Only one of five columns renders cards.** `bee_classify_features`
  (`views.rs:2617`) places every feature into **Todo · In Progress · Review ·
  Compound · Finished**, but `bee_hub_card` (`views.rs:3655`) runs for In
  Progress alone — Todo, Review, Compound and Finished render one-line rows via
  `bee_hub_finished_row` (`views.rs:3910`). The desktop screenshot shows four
  columns of cards; the user chose to keep the dense rows and take the console
  styling only, so this stays a restyle and the render path is untouched.
- **The Geist faces are obtainable and cheaper than what ships today.** The
  variable latin-subset woff2 files are 29.4KB (Geist) and 23.1KB (Geist Mono)
  against the 302.7KB of base64 that `fonts.css` currently spends on static
  Manrope and JetBrains Mono weights. Replacing both faces shrinks the served
  bundle by roughly 230KB.
- **The store's real surface is narrower than the screenshots.** Available per
  card: cell counts (`BeeFeatureCellCounts`, `bee.rs:713`), per-cell proof
  verdict (`BeeCell.tests`, `bee.rs:140`), live worker names
  (`BeeRunningWorker`, `bee.rs:347`), worktree branch (`BeeWorktree.branch`,
  `bee.rs:607`), last activity (`BeeState.last_activity`, `bee.rs:190`), run
  state, deferred-debt count, archived count and ship time
  (`BeeArchivedFeature`, `bee.rs:1248`). **Not available at all**: PR number or
  state, comment counts, avatars, CI checks, merge-readiness as a stored
  verdict — confirmed by repo-wide search, no such field exists in any struct.
- **The responsive guard is a single-media-query assertion.**
  `bee_hub_style_puts_in_progress_order_rule_only_inside_the_narrow_media_query`
  (`views.rs:11708`) asserts `@media (max-width: 700px)` appears exactly once.
  D3 adds breakpoints, so this guard is deliberately re-shaped to keep its
  intent (the ordering rule stays narrow-only) while allowing more than one
  query.

Evidence commands: `cargo test -p waggledance` for every pinned literal above;
`rg 'prefers-color-scheme' crates/` returns only the negative assertion.

## Approach

**Recommended path.** Swap the theme adapter first and prove it alone (D1),
then rebuild the board structure on top of a theme that is already correct.
Restyling markup and swapping the palette in one step would leave every failing
CSS-literal test ambiguous between the two causes.

The console palette is expressed as Tier-1 private primitives in a new theme
file and mapped onto the **existing** contract token names — no token is added,
renamed or removed, so the published contract survives untouched and the
`.fg-*` component layer needs no edit. The digest's theme-invariant status
colours become contract-legal tokens rather than raw hexes at their use sites.

The board's ten per-project identity hues are *not* part of the deleted palette
override: they are identity, not theme, and are re-expressed in console values
while keeping their existing rule shape and test.

**Rejected alternatives.**

- *Edit `atelier.css` in place.* Loses the ability to diff old against new and
  destroys the reference theme the design system ships as an example.
- *Add a second theme and let `data-theme` pick.* D1 says waggledance ships one
  theme; a picker is scope nobody asked for and doubles the surface to test.
- *Restyle the board with page-local CSS.* Exactly the override D1 deletes.
- *Match the screenshots' PR chips, comment counts and avatars from a new data
  source.* Out of scope and against D2 — the cockpit reads bee's store only.

**Risk map.**

| Component | Risk | Proof needed |
|---|---|---|
| Theme adapter swap | MEDIUM — every surface changes at once | Full `-p waggledance` suite; the bundle carries no `prefers-color-scheme`; both schemes render |
| Typeface replacement | MEDIUM — the mono face is pinned by a test and every surface re-flows | Rewritten test pins Geist Mono; the bundle embeds no external font URL |
| Deleting the page-local override | MEDIUM — a pinned test asserts its literals | Rewritten test asserts the board carries no palette block and inherits contract tokens |
| Phone breakpoints | LOW-MEDIUM — the single-media-query guard | Re-shaped guard still pins the ordering rule to the narrow query |
| Contrast and colour-only meaning | MEDIUM — dark, low-chroma palette | Every status dot keeps its text label; contrast checked at ship values |

## Shape

| Phase | What Changes | Why Now | Demo | Unlocks |
|---|---|---|---|---|
| **1. Typefaces** | Geist and Geist Mono replace Manrope and JetBrains Mono as embedded variable latin woff2; `--font-*` tokens re-pointed; the mono-face test rewritten | The type scale is half of the console look, and every later phase is judged against text that must already be the right face | Every page renders in the console faces, offline, with no external font request | The type scale phases 2-4 apply |
| **2. Theme adapter** | New console theme file replaces `atelier.css` in the bundle; the board's page-local palette block is deleted; pinned palette tests rewritten | D1's leverage point — one file changes every surface, and it must be provably correct before markup moves | Every page — board, project pages, doc reading pages — renders dark console in both schemes | All later phases restyle against a correct palette |
| **3. Board surface** | Console column headers (status dot, label, right-aligned mono count); the Finished column becomes a collapsed `ARCHIVE n` bar spanning the board; the In Progress card and the dense rows restyled to the digest under D2 | The board is the surface the screenshots are of; it needs a correct palette and face underneath it first | The desktop board reads as the screenshot's board, showing only what bee knows | Phone sections reuse the same markup |
| **4. Phone layout** | Breakpoints over the same markup: stat tiles, grouped sections, bottom-anchored chrome | D3 is responsive CSS over phase 3's markup — it cannot precede it | The homepage at phone width matches screenshot 2 | — |
| **5. Spec re-sync** | `docs/specs/bee-cockpit.md` updated from its stale three groups to what shipped | CONTEXT.md requires it as part of this feature | The spec describes the board that exists | — |

### The two questions CONTEXT.md carried into planning

- **Column-to-status-colour mapping.** Five waggledance columns map onto the
  digest's four status colours plus the archive bar: **Todo** → working blue
  (the screenshot's "Pending Work"), **In Progress** → iterating orange,
  **Review** → in-review yellow (no glow, per the digest), **Compound** →
  ready-to-merge green, **Finished** → the archive bar in muted, not a colour
  of its own. The mapping is one-to-one with the screenshot; Compound is the
  honest local meaning of "ready to merge" because it is the stage after work
  is capped and before the feature closes.
- **Finished as the collapsed `ARCHIVE n` bar.** Yes. It is the shape the
  screenshot shows, and it is also what the cockpit spec already asks for — a
  finished list collapsed by default that states its true count while closed.
  The bar spans the board under the four columns and expands to the same paged
  rows that exist today, so nothing is dropped, only folded.

### Two shape decisions the user made at this gate

- **Typefaces are replaced, not approximated.** Geist and Geist Mono ship
  embedded; Manrope and JetBrains Mono are dropped. The variable latin subsets
  cost 52.5KB raw against 302.7KB of base64 today, so the bundle gets smaller.
- **The dense rows stay dense.** Todo, Review and Compound keep their one-line
  rows rather than becoming full cards. The board will not match the
  screenshot's four-columns-of-cards shape, deliberately: those columns grow
  without bound and density is worth more there than fidelity. Only the In
  Progress card carries the full card anatomy.

### Current slice — phases 1 and 2

Three cells, serial — each touches the bundle or `views.rs`, so they cannot
overlap:

1. **Faces.** Embed Geist and Geist Mono as variable latin woff2 in
   `fonts.css`, drop Manrope and JetBrains Mono, re-point `--font-display`,
   `--font-body`, `--font-mono`, `--font-accent` and `--num-font`, and rewrite
   `served_stylesheet_bundles_jetbrains_mono_and_leads_font_mono_token` to pin
   Geist Mono and to keep asserting that no `url(http` reaches the bundle.
2. **Theme.** Author the console theme file, map the digest's dark and light
   schemes onto the existing contract token names through `data-scheme`, and
   put it in the bundle in place of `atelier.css`.
3. **Board palette.** Delete the page-local palette override, re-express the
   ten project identity hues in console values, and rewrite
   `feature_hub_theme_tokens_render_for_both_light_and_dark` to assert the
   board carries no palette block and inherits the theme.

Phases 3-5 stay one-line headlines above until this slice caps.

## Test matrix

The triad at its smallest demonstrating size; each cell's writer audits
existing coverage first and authors only the gap.

- **Happy path** — the served bundle carries the console theme's tokens in both
  `data-scheme` values; the board page renders with no page-local palette
  block; a board card renders its store-backed elements.
- **Edge cases** — a feature with zero cells renders no progress element rather
  than `0/0`; a feature with no worktree renders no branch row; an empty board
  still renders the tab strip and its honest empty state; the archive bar
  states its true count while collapsed.
- **Error paths** — an unreadable `.bee` file still degrades to the warning
  strip; a malformed timestamp still renders the card without an activity line.

Three existing guards are deliberately re-shaped rather than deleted, each with
its reason recorded on the cap: the mono-face test (its subject face is
replaced, and its real intent — a self-contained bundle with no external font
URL — is kept and strengthened), the palette-literal test (its subject is
deleted by D1), and the single-media-query test (D3 adds queries; the guard's
real intent — the ordering rule stays narrow-only — is preserved).

## Out of scope

- Any PR, comment, avatar, or CI-checks element from the screenshots — no data
  source exists and D2 forbids inventing one.
- The mobile `+` FAB and the `Merge PR` control — the cockpit is read-only.
- A theme picker or any second shipped theme.
- Full cards in the Todo, Review and Compound columns — the user chose density
  over matching the screenshot's card grid there.
- The per-project board at `/p/:id/_bee` beyond what it inherits from the
  shared theme and shared card renderer.
