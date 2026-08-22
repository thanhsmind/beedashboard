# Agent-orchestrator style digest

Distilled read-only from `/home/thanhsmind/projects/AI/agent-orchestrator`.
Every value below is what that project actually ships. Reproduce values through
waggledance's own token contract — never hard-code a hex a token can carry.

## Token architecture there

Dark-first. `:root` is dark; `:root[data-theme="light"]` overrides. Tiers:
shadcn primitives in `oklch()` → semantic `--color-*` roles → `--bridge-*`
re-exports → component-scoped tokens → legacy aliases. Tailwind v4 `@theme
inline` maps the utility scale onto those names.

## Dark palette (the native scheme)

| Role | Value |
|---|---|
| page background | `oklch(0.185 0.006 285.885)` |
| text primary | `oklch(0.985 0 0)` |
| card / panel surface | `oklch(0.24 0.008 285.885)` |
| popover / raised | `oklch(0.28 0.008 285.885)` |
| sidebar (cooler, darker than page) | `oklch(0.155 0.005 285.823)` |
| sidebar active row fill | `oklch(0.274 0.006 286.033)` |
| muted surface | `oklch(0.274 0.006 286.033)` |
| text secondary | `oklch(0.705 0.015 286.067)` |
| text passive (timestamps, meta) | `oklch(0.442 0.017 285.786)` |
| primary (button fill — near-white, not blue) | `oklch(0.92 0.004 286.32)` |
| primary foreground | `oklch(0.21 0.006 285.885)` |
| border hairline | `oklch(1 0 0 / 7%)` |
| divider / strong border | `oklch(1 0 0 / 4%)` |
| focus ring | `oklch(0.552 0.016 285.938)` |
| hover overlay | `color-mix(in oklch, var(--foreground) 4%, transparent)` |
| active overlay | `color-mix(in oklch, var(--foreground) 7%, transparent)` |
| destructive | `oklch(0.704 0.191 22.216)` |

## Light scheme

background `oklch(0.985 0 0)` · foreground `oklch(0.141 0.005 285.823)` ·
card `oklch(0.985 0 0)` · popover `oklch(1 0 0)` · primary
`oklch(0.21 0.006 285.885)` · muted `oklch(0.967 0.001 286.375)` ·
muted-foreground `oklch(0.552 0.016 285.938)` · border
`oklch(0.945 0.003 286.32)` · input `oklch(0.92 0.004 286.32)` · sidebar
`oklch(1 0 0)`.

## Status colours — theme-invariant, identical in light and dark

| Meaning | Hex | Dot glow |
|---|---|---|
| working / pending work | `#60a5fa` | yes |
| needs you / iterating | `#fb923c` | yes |
| in review | `#facc15` | **no** |
| ready to merge / merged | `#4ade80` | yes |
| exited / failed | destructive | — |
| idle | muted-foreground | — |

Glow spelling: `color-mix(in srgb, <status> 60%, transparent)` as a blurred
box-shadow behind the dot. Ambient per-column glow radius `130px` at
`color-mix(in srgb, <status> 7%, transparent)`.

Card-local PR-state colours: approved `#86efac`, in review `#fcd34d`, changes
requested `#fdba74`, open `#9ca3af`. Activity colours: passed `#86efac`,
failed `#f87171`, reviewing `#93c5fd`, waiting `#fdba74`, default `#9ca3af`.
Progress-ring track `#374151`; ring fill green `#4ade80` at 100%, orange
`#fb923c` below 50%, `#e5e7eb` between.

Attention border on a waiting card: review `#f87171` at 70% alpha, otherwise
`#fb923c` at 60% alpha, 1px.

## Type

- UI face: Geist Variable, then `ui-sans-serif, system-ui, sans-serif`.
- Mono face: Geist Mono Variable, then the usual nerd-font/`ui-monospace` stack.
- Sizes (dense, with half-steps): 8, 10, 10.5, 11, 11.5, 12, 12.5, **13
  (dense controls, the base)**, 14, 15, 16 (brand/board title), 17, 21, 22.
- Weights: only 500 and 600 are tokenised; 400 is the bare default.
- Line heights: 1.5 normal, 1.42 snug, 1.55 body, 1.6 relaxed, 1.7 loose.
- Tracking runs `-0.025em` to `0.12em`.

Element assignments on the board: card title 11.5–12.5px semibold, tight
leading and tracking · branch line 9.5–10.5px **mono** · meta and badges
10–10.5px · column header 10.5px medium (the real product renders it **mono,
uppercase, wide-tracked**; the marketing mockup simplifies it to sans) ·
board/project title 16px semibold · pill buttons 12px semibold.

## Geometry and depth

Radius derives from a 10px base: 4 / 6 / 8 / **10 (cards)** / 14 (panels) /
999 (pills, avatars, dots); 2px for square lane swatches. Window shell 20px
with a 17px inner surface — the nested-corner relation comes from a
window radius minus a 6px inset.

Borders are 1px hairlines everywhere; nothing is thicker.

Spacing is a 4px base *with half-steps*: 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
13, 14, 15, 16, 18, 20, 22, 24, 32, 40.

Elevation is nearly flat: cards carry `0 1px 1px rgba(0,0,0,0.05)`. Only the
window shell gets depth (`0 30px 80px -24px rgba(0,0,0,0.75)`).

Board metrics: four equal fluid columns divided by 1px rules, column header
48px tall with 16px side padding and 10px between dot/label/count, column body
12px padding with 10px between cards. Card internals: header row 14px sides,
12px top, 10px bottom, 10px between icon and title; a hairline divider inset
14px; meta block 14px sides, 8px vertical, 6px between rows. Dots are 7px.
Avatars 18px, overlapping by 6px, each ringed 1px in the card colour.

Motion: 120ms fast, 150ms normal. Card enter/exit 220ms on
`cubic-bezier(0.22, 1, 0.36, 1)`; layout reflow 250ms on the same curve;
archive expand 140ms on `cubic-bezier(0.25, 0.46, 0.45, 0.94)`. Status dots
breathe; the waiting-card border pulses on a 1.2s loop
(`0 0 0 0 rgba(251,146,60,.35)` → `0 0 0 4px rgba(251,146,60,0)`), disabled
under `prefers-reduced-motion`.

## Desktop board anatomy (screenshot 1)

**Column header**, in order: status dot (7–8px, round, glowing except in
review) · label · count pushed right in mono at 60% opacity · optionally an
`N waiting` chip in `#fb923c` on a 10%-alpha fill of itself.

**Card**, in order:
1. Header row — a small agent/harness mark (14px), optionally overlaid at its
   top-right by a 10px `!` badge when the card wants attention; then the title.
2. Meta block — a branch row (fork glyph + mono branch name, truncated); then
   a PR row (fork glyph + mono `#123` + state word, coloured by state).
3. Social row — overlapping avatar stack, then `38/60 tests`, then a
   right-aligned comment count.
4. Footer, above a hairline — either a `Merge PR` primary pill with the
   relative time on the right, or an activity glyph (check, warning, spinner,
   or a 12px progress ring) plus its text (`Checks passed`,
   `38/60 passed`, `Debugging issue`) with the relative time on the right.

The live row in the shot is that last variant: a spinning ring next to
`Debugging issue`, `now` on the right.

Columns in the shot: Pending Work (blue) · Iterating (orange) · In Review
(yellow) · Ready to merge (green). A collapsed `ARCHIVE 72` bar sits under
the whole board.

**Sidebar**: window controls · brand mark and name · a search field
(28px tall, muted fill, rounded) · a `Pinned` group · a `Projects` group with a
trailing `+` · the active project row filled with the sidebar-accent colour and
carrying trailing action icons · child rows each with a 6px status dot ·
`Settings` pinned to the bottom.

**Topbar**: 40px tall over a hairline, 16px sides — title at 16px semibold,
then a spacer, then a bordered `+ New task` pill, a filled `Orchestrator`
pill, and a 32px bell button.

The board sits inside a sidebar-coloured frame with an inset, separately
rounded and bordered inner surface.

## Phone anatomy (screenshot 2)

Its own hex ramp, close in spirit but numerically distinct: base `#0a0b0d`,
surface `#121317`, elevated `#15171b`, hover `#191b20`, text `#f4f5f7` /
`#9ba1aa` / `#646a73` / `#444951`, borders `rgba(255,255,255,.06)` and
`rgba(255,255,255,.10)`, blue `#4d8dff`, orange `#f59f4c`, amber `#e8c14a`,
green `#74b98a`, purple `#a78bfa`, red `#ef6b73`; status tints at 14% alpha.

In order: a header with the screen title at 22px extra-bold, a connection lamp
and a mono host line at 9.5px, and a bell with an unread dot · three stat
tiles, each a big mono number at 19px extra-bold over a 9px semibold label,
coloured when non-zero and faint at zero — `working`, `need you`,
`mergeable` · a project switcher · section headers, each a 2.5px coloured
vertical bar plus a 9px bold uppercase label tracked at 1.1px with a
right-aligned mono count · rows carrying an agent mark, a bold title, a mono
branch line, a hairline, then a status pill of coloured breathing dot plus
status word with the relative time on the right · a bottom tab bar of four
items, the active one tinted blue with a heavier stroke and an 8.5px semibold
label · a 38px blue `+` FAB bottom-right.

## Icons

lucide-react in the product; the marketing mockup hand-draws the same glyphs
inline as stroke SVGs (1.2–1.8 stroke, round caps, 16px box) to keep a
decorative subtree out of the tab order. Custom glyphs not in lucide: the
orchestrator mark (four connected nodes), the branch and pull-request fork
glyphs, the panel glyph, and an hourglass-ish waiting glyph.
