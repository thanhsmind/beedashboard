---
area: bee-cockpit
updated: 2026-08-13
sources: [feature-close, agent-board, bee-artifact-rename, archive-visibility, feature-hub, board-declutter, board-trim, feature-titles, hub-fallbacks, detail-desc-wrap, cross-board, board-drop-live, card-terminals]
decisions: []
coverage: partial
---

# Bee Cockpit

A read-only surface inside mdview that shows what the bee harness is doing in a
registered project: which features are waiting on a person right now, which are
in progress, which have already shipped, and the backlog and review queue
behind any of it.

Throughout this surface's own pages, the name shown to a person is **Bee
Artifact** — page titles, the top bar, and other on-page UI text all read that
way. This is a display label only: the command a person types, and the
identifiers this product exposes to other tools, remain **mdview**.

Technology-agnostic: this describes behavior and rules, not the Rust that implements
them. Code entry points are listed in `reading-map.md`.

## Who it is for

Someone running bee across several projects who cannot read bee's own store. bee keeps
a thorough record — cells, lanes, sessions, backlog, reviews, decisions — spread across
JSON and JSONL files in each project's `.bee/` directory. That record is precise and
unreadable. This surface answers, in plain language, the questions a project manager
actually asks: what has been built, what is being worked on, what comes next, and where
is it stuck.

## Where it appears

A project qualifies for the bee surface when **both** hold:

1. It is registered with mdview (it appears in the project registry).
2. Its root directory contains a `.bee/` directory.

A registered project without `.bee/` behaves exactly as it always did — opening it
still goes straight to its entry document. No bee link appears, and requesting a bee
page for it returns not-found rather than an empty bee page. The absence of a store is
never rendered as an empty dashboard.

A qualifying project gains an entry point on its home page leading to its board.

## The cross-project board

The same surface also exists once for everything at once. The viewer's own front
page — the page a person lands on before choosing a project — leads with the
Features board rolled up across **every** qualifying project, and only then shows
the list of registered projects that page has always shown:

1. **Features**, across all projects.
2. The project list, unchanged — same rows, same order, same everything below it.

There is deliberately no cross-project Live strip. Presence across many projects
answered nothing the Features columns did not already answer, while taking the
top of the page to do it; the Live strip stays where it earns its place, on a
single project's own board, where it answers who is at work in *this* project.

Qualification is the same rule stated above, and it decides only what feeds the
Features section: a registered project without a `.bee/` store still appears in
the list below, exactly as before. When no project qualifies at all, the section
is not built and the front page is precisely the page it was — a person with no
bee projects is never shown an empty frame.

The rolled-up board reads the same way its per-project counterpart does, with
three differences that follow from having more than one project in view:

- Every entry names the project it belongs to. Without that, a merged list is
  unreadable.
- The lists stay **flat**. Features are never grouped into per-project blocks and
  a project never gets a row of its own. The question this page answers first is
  "what is waiting on me", and the project is a detail of the answer, not the axis
  it is organised by.
- Ordering, counting, and truncating all happen **after** the merge, over the
  combined sequence — never per project and then stitched together.

Reading every project's store is more work than reading one, so it is done off to
the side and for all projects at once, never one after another while the page
waits. The per-project board is untouched by any of this: it keeps its own address
and renders exactly what it rendered before.

## The reading order — and why it is the feature

The board answers, top to bottom, in a fixed order:

1. A top bar naming the project and the instant this snapshot was read.
2. **Live** — a single dense strip of what is running at this moment, one line each.
3. The **Feature Hub** — one card per feature, grouped by whether it is waiting on a
   person, still in progress, or finished.
4. **Finished (shipped) list** — the complete, permanent record of every feature that
   has fully shipped, collapsed by default so it never crowds out the live work above
   it.
5. **Backlog & Review** — the backlog of proposed work and the state of every review
   candidate, for the reader who wants to go one level deeper than the headline view.

This order is not decoration; it is the feature. It mirrors the sequence a project
manager actually asks the questions in: first "what needs me and what's moving right
now," then "what's already done," and only after that, backlog and review detail. A
section with nothing to show never disappears and never disturbs this order — it
renders its own honest empty line in its place (see "Honesty rules that hold
everywhere," below).

## Live

Between the top bar and the Hub sits one narrow presence strip answering a single
question — what is running right now:

- One line per live session, naming the lane it is working, that lane's current stop,
  how long ago it last reported in, and the checkout it is working in (the main
  checkout is named as such rather than left blank). A session with no lane of its own
  is labelled by the project's active feature, or plainly as having no active lane.
- One line per branch checkout, naming the branch and the feature it carries, or
  saying it carries none. A checkout that could not be read says so and why, rather
  than being dropped or guessed at.
- Nothing running at all → one line saying so. The strip never disappears.

This strip is deliberately thin: it carries presence, not detail — no worker rosters,
no per-file contention, no health scoring. Those readings still exist for the drill-down
pages; this section is the glance that tells a reader whether anyone is at work before
they read anything else.

## Feature Hub

The board's main section is feature-centric: one card per feature, never one card per
cell. Every feature the store knows about — whether it still has live work or has
already shipped — is placed into exactly one of three groups:

- **Waiting on you** — a feature whose live work is sitting at a gate that has not yet
  been approved, or that carries a paused handoff note. Two things never place a
  feature here: the independent-review gate never counts as a stop (see "Independent
  review is always invoked," below), and a handoff explicitly recorded as a clean,
  already-claimed handover to the next piece of work — a `planned-next` handoff — never
  counts as waiting either.
- **In Progress** — every other feature that still has live work and is not waiting on
  a person.
- **Finished** — a feature that has either fully shipped (compounding complete) or been
  archived. A card in this group carries a chip naming the state of its worktree: still
  the main checkout, an open worktree, or one already merged.

A card shows the feature's human title with its slug as a subtitle beneath it, its
Feature Boundary sentence, and a description. The title and description are read from
the first of these that exists: the feature's own `CONTEXT.md` heading and boundary
text, then the most recent decision scoped to the feature, then the title of its first
cell. A long description wraps and is visually clamped — on the card, and on the
feature's own detail page too — rather than overflowing or forcing either page to
scroll sideways. A card links onward to the feature's own detail page.

### The terminals running behind a card

On the cross-project board, a Waiting on you or In Progress card also carries one
marker per terminal session running in that feature's own checkout, each showing
that session's state and the program it runs, and each opening that session's own
terminal view. Finished rows carry none.

Which sessions those are is decided by the checkout, not by the feature, because
nothing in the record ties a session to a feature: a session knows which feature
it is working, but not which terminal it occupies, and a terminal knows which
directory it sits in, but not which feature. The directory is the only thing both
sides share. So:

- A feature with its own branch checkout is marked with the sessions running
  inside that checkout, and with no others. For such a feature the marking is
  exact.
- A feature working in the project's main checkout is marked with the sessions
  running there and in no branch checkout — which every other main-checkout
  feature of that project shares. The markers are therefore labelled as the
  terminals of *this checkout*, never as the feature's own; claiming otherwise
  would be a lie the reader could not check.

A feature with no session in its checkout shows no marker and no empty frame for
them. When the terminal switch is off, or the session host cannot be reached or
does not answer promptly, no card carries a marker and the board renders exactly
as it otherwise would — the same rule the project list already follows. The
per-project board carries no such markers.

### How each group is ordered, and how Finished is cut short

Within the Waiting on you and In Progress groups, features are listed by name.

Finished is different, because it is the group that grows without bound — a
long-running project accumulates hundreds of finished features while the other two
groups stay small. Two rules keep it readable:

- **Order.** Finished features that carry a ship time come first, most recently
  finished at the top, each showing that time. Every remaining finished feature
  follows, listed by name. A feature has a ship time only when the whole of its
  finished work is timed; a feature whose record is partly timed counts as untimed
  rather than being placed on a half-known date. On the cross-project board this
  is one shared sequence across every project, not one sequence per project.
- **Length.** The group shows ten entries and puts the rest behind a control that
  names how many are hidden and reveals the next ten at a time. Nothing is ever
  dropped — only folded. On the cross-project board the ten and the hidden count
  are both taken from the merged total.

## Finished (shipped) list

Every feature that has fully shipped, collapsed by default behind one summary line
that states the true count of finished features and finished cells even while the
list itself stays closed — collapsing a list is never allowed to understate what it
holds. Opening it shows one compact line per feature — never one card per cell —
naming its cell count and, when both of its timestamps are on record, its time to
finish. This list is never capped or truncated: every feature that has shipped is
named here, no matter how many there are. A project with nothing finished yet shows a
single honest line instead of a collapsible, zeroed list.

A feature that has fully shipped appears twice on the board by design: once as a card
in the Feature Hub's own Finished group, and again as a line in this list. The two are
not competing claims about the same thing — the Hub groups every feature, live or
shipped, by its current status at a glance, while this list is the complete, standing
record of everything that has ever shipped, and is the one place that record is
guaranteed never to be capped.

## Backlog & Review

Three sub-views in one supporting panel:

- **PBIs by status.** Backlog work items are event-sourced — the same item can appear
  many times as its status changes over time, and its **current** status is whatever
  its most recent recorded entry says. The browsable list shows only **open** items —
  anything already done or declined is left out of the list itself — capped at a
  recent 20; a project with more open items than that shows the true total of open
  items alongside the visible subset ("Showing X of Y") rather than looking smaller
  than its real open backlog. The status chips above the list still carry the full
  count for every status, done and declined included, so closed work is never erased
  from the numbers, only from the scrollable list. Each item can be expanded to reveal
  a further line of its own detail. A project with no open items — whether because it
  has no backlog at all, or because everything in it is already done or declined —
  says so plainly with its own empty state.
- **Findings by severity.** Each recorded finding carries a severity of P1, P2 or P3;
  they are summarised by severity, with P1 given visual weight because a P1 finding
  blocks, and the same bounded-recent-slice-with-true-total treatment as PBIs. A
  project with no findings yet says so plainly.
- **Review queue by state.** Every review candidate the store has recorded is placed
  into exactly one of three states by joining it against every recorded review
  session: **unreviewed** (it has never appeared in any session), **in review** (it
  appears in a session whose decision has not yet settled), or **settled** (it appears
  in a session whose decision reached approved or blocked). The count of open P1
  findings is called out first. Every sentence in this panel words independent review
  as something the project's owner invokes — nothing here ever implies review is
  already running or already queued as pending automatic work.

A store with **zero** recorded review candidates is genuinely ambiguous — it is the
same shape whether a project has never run a review at all, or whether every candidate
has already been folded and rolled off the list — so this one case says review state is
unknown rather than rendering three zeroes as if they were a real measurement. From the
moment even one candidate is recorded, every count is real and computed, including a
genuine zero for "in review" or "settled" when every recorded candidate really is
unreviewed.

## Drilling in

Every feature card links to its own feature detail page.

- A **feature detail page** shows the same human title, slug subtitle, Feature
  Boundary sentence, and clamped/wrapped description as its Hub card, alongside a row
  of chips summarising its current state. A **Docs row** links to every markdown file
  recorded under the feature's own history docs. Below that sit three tabs:
  **Activity**, **Todos**, and **Terminal** — the Terminal tab lists the project's own
  live agent-terminal panes; a plain worker list is no longer shown anywhere in this
  UI.

  A feature's own detail page is not archive-free the way the Hub and the Finished
  list are: it merges a feature's archived cells (the record `bee close` produces)
  together with its still-live ones. A feature whose only remaining cells are archived
  still shows correctly — it reads Closed, and its done-cell count includes the
  archived ones rather than reading zero.
- A **cell page** shows that cell in full: what it is, what proves it, its state and
  lane, the files it touches, the decisions it cites, its required outcomes, and its
  whole execution trace — who ran it, when it was claimed and capped, its outcome,
  recorded deviations, and its test result. Cells are not surfaced directly on the
  board any more; each is reached through its own feature's detail page. A cell page
  resolves an archived cell exactly as it does a live one.

An unknown cell or feature name returns a clean not-found, never a blank page.

## Two guarantees that make this board safe to point at a project

### It never writes to a project's store

The surface never writes to a project's `.bee/` directory. It approves no gate, claims
no cell, edits no backlog item, and ends no session. Those actions belong to the bee
CLI and to the live sessions that own that state; a dashboard that wrote there would
race a running agent.

This is enforced, not merely intended: the project's entire `.bee/` tree — every file
and every directory in it — is compared byte for byte before a request and after it,
for every kind of page this surface renders. A caching layer, or any code path that
merely listed a directory in a way that could create one as a side effect, would fail
that check.

### It renders nothing that identifies a filesystem outside the project

This surface carries no authentication of its own — nothing in mdview does, including
the agent terminal (see the Agent terminal spec) — and can be bound to a non-loopback
address. bee's store is full of absolute paths — the files a cell touches,
a worker's identity, a session's transcript, a workspace root. **None of them may reach
the page.** A field that is itself entirely a path is rendered relative to the project
root, or dropped.

Free text is a harder case, and this board holds the guarantee there too: several
fields the board renders are free-form prose from the store — a feature's own boundary
sentence and description, a review finding's own description, a backlog item's own
detail — and any of these can have an absolute path typed into the middle of a
sentence, not as the whole field. Every one of those fields is scanned for an absolute
path embedded anywhere inside it, and any path found is reduced the same way a
wholly-path field would be, while the words around it survive untouched. A project's
operator can write "see /home/them/notes.txt for context" into a feature's description
and that sentence still renders, with the path portion alone reduced.

A string shaped like an application route rather than a filesystem path — for example
`/p/:id/_bee` — is not treated as a path by this scan and is left exactly as written;
only a string that resolves as a genuine absolute filesystem path is reduced.

A feature name is itself free text from the store, and this board has to join a
feature name onto a location on disk to list the markdown files recorded under that
feature's own history docs, for the Docs row on its detail page. Before any such join
happens, the name is validated: no path separators, no `..` segment, no leading dot,
no control characters, and not already an absolute path of any shape. A name that
fails validation is never looked up at all — the join is never attempted, so a
maliciously- or accidentally-shaped feature name can never make this board read, or
claim to check, anything outside the project it was asked about.

The tests that guard both halves of this guarantee assert against the **fixture's own
root path**, not against a literal that merely looks like a production path — a check
written against one hardcoded prefix would pass green while a real page leaked a real
path verbatim. See `docs/history/learnings/20260805-toothless-security-assertions.md`.

## Honesty rules that hold everywhere

Four rules apply across every section of this board, not just the ones above that
happen to illustrate them:

- **A dropped cell counts toward no total and no denominator, anywhere.** Not in a
  feature's own progress, not in its cell count on the Finished list, not in whether a
  feature counts as shipped. It never shipped, so counting it as done would inflate
  the picture; it is simply absent from every count that would otherwise include it.
- **A capped or truncated list always states its true total beside the visible
  subset.** A real store can be large — hundreds of backlog rows and findings are
  normal — so detail lists are bounded to a recent slice, and whenever that slice is
  smaller than the true total, the panel says so. The one list on this board that is
  never capped is the finished-features list — nothing that has shipped is ever
  silently left off it.
- **Nothing to measure renders as a stated absence, never as a zero or a division
  artifact.** "No features have shipped yet," "no live cells recorded for this feature
  yet," "no open backlog items yet" — these are the shape this rule takes. A number
  that is genuinely, computably zero — a bucket that really does hold no cells right
  now, or a feature whose real, measured time to finish happens to round to a very
  small figure — is not what this rule forbids; it forbids manufacturing a number
  where there was no measurement to take.
- **A store that cannot be fully read says so.** Any single unreadable file — missing,
  empty, truncated, or malformed — degrades the page to a partial view that names what
  could not be read, rather than silently dropping or miscounting the data it held. It
  never takes down the page, and a malformed line among otherwise-good lines loses
  only itself. A project with no store at all is a different, earlier case — presence,
  not degradation — and is a clean not-found, never an empty dashboard (see "Where it
  appears," above).

## Independent review is always invoked

Wherever this board mentions independent review — the Feature Hub's definition of
"waiting on you," and the review queue panel in Backlog & Review — it is worded the
same way: review is something the project's owner invokes, never a stage the board
implies is already running, already queued, or pending on its own. A gate that has not
yet been through review reads as "not yet approved," never as "review in progress"; a
candidate that has never appeared in a session reads as "never reviewed," never as
"awaiting automatic review." This holds even when the count of unreviewed work is
large enough that a different phrasing might read as more urgent — the wording never
implies the board itself is doing, or about to do, that work.

## Bounded output

The snapshot is rebuilt on every request, and a real store is large — hundreds of
backlog rows and thousands of decision events are normal. Detail lists are capped at a
small recent slice, and each panel states its true total when it is showing a capped
subset (see "Honesty rules that hold everywhere," above).

The Feature Hub and the standalone Finished list are archive-free: only live cells
feed a feature's progress and its shipped status there. A feature's own detail page,
and a cell page, are not — see "Drilling in," above.

## Scope

This surface covers **one project at a time**. A single page aggregating every
registered project — total active projects, velocity across all of them, which lanes
run where across the fleet — is a separate, later feature.
