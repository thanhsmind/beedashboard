# Bee Cockpit

A read-only surface inside mdview that shows what the bee harness is doing in a
registered project: where the active feature sits in its own lifecycle, how much has
landed, what is happening right now, what needs a human, how fast work is shipping,
what is in flight broken down by stage, what has already finished, and the backlog,
sessions and process detail behind any of it.

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

## The reading order — and why it is the feature

The board answers, top to bottom, in a fixed order:

1. A header naming the project and the instant this snapshot was read.
2. The **lifecycle stepper** — where the active feature sits between exploring and
   independent review.
3. **Headline numbers** — how much live work is in each state, and how many features
   have shipped in total.
4. **Working on now**, beside **needs attention** — what is happening this minute, next
   to what of that deserves a human's eyes, so the two are read together rather than
   one being buried below a scroll of everything else.
5. **Delivery speed** — how fast the project is shipping.
6. **Work by phase** — every feature still in flight, grouped by the stage of the bee
   lifecycle it is in.
7. **Finished** — every feature that has fully shipped, collapsed by default so it
   never crowds out the live work above it.
8. Supporting panels — backlog and review queue, where work is happening, and process
   health — for the reader who wants to go one level deeper than the headline view.

This order is not decoration; it is the feature. It mirrors the sequence a project
manager actually asks the questions in: first "where does this sit in its own
lifecycle," then "how much has landed," then "what's happening right now and what of
that needs me," then "how fast is this moving," then "what's in flight and how far
along," then "what's already done," and only after all of that, supporting detail. A
section with nothing to show never disappears and never disturbs this order — it
renders its own honest empty line in its place (see "Honesty rules that hold
everywhere," below).

Older revisions of this board also grouped every cell into four buckets by cell
state — "what is being worked on, what is waiting, what is stuck, what is done" — as
its own top-level section. That grouping is gone from the board: a project manager
asks what feature is being built and how far along, not what state an individual cell
carries. The four cell states still matter — they are what the headline numbers count,
what a phase card's progress bar is built from, and they still appear as their own
four-bucket view, one per cell, on each feature's own detail page (see "Drilling in,"
below) — they are simply no longer their own section of the board itself.

## Lifecycle stepper

Four steps, always in this order: explore, shape, execute, independent review. A step
reads **done** when its gate is currently recorded as approved — full stop. A gate that
was approved and later revoked, and has since been approved again, reads as approved
today; today's record beats a stale revocation. The first step that is not done is the
**current** one.

An undone step's own note distinguishes two different histories: a gate with no
approval on record ever reads "not yet approved," while a gate that was approved and
then explicitly revoked reads "approved, then revoked" — it was taken away, not merely
never reached. The independent-review step is worded differently from every other step
regardless of which of those two histories it carries: it always reads that it runs
only when a human invokes it, never automatically. This is deliberate — a stepper that
told a passerby "review: not yet approved" would misrepresent review as pending
automatic work, exactly what this board must never imply (see "Independent review is
always invoked," below).

A project with no lifecycle record at all renders one honest line instead of four
steps all reading "not yet approved" — the absence of a record is not the same claim as
a record that positively says no.

## Headline numbers

Five counts, side by side: how many cells are being worked on right now, how many are
waiting and unclaimed, how many need a human, how many are finished, and how many
features have shipped in total. Every one of these is a real, honest count — a bucket
that genuinely holds zero live cells renders `0`, which is real data, not the "nothing
to measure" case the honesty rules protect against elsewhere on the board. A cell
whose own state this board does not recognise, and a cell that was dropped before it
ever shipped, are counted in none of the four state tallies (see "Honesty rules that
hold everywhere," below).

## Working on now, beside needs attention

### Working on now

Names the one feature currently active, drawn only from the project's own recorded
state — never guessed from cell data, which would risk the same feature being named a
second time down in the finished list once it ships. Alongside its name: the recorded
rationale for why the project is being worked the way it is, progress over that
feature's own live (non-dropped) cells, and its recorded next action. A card also
carries its own "Running now" list — every live worker the store currently knows about,
each naming the cell it is on. When the running worker names a cell the store cannot
find, or the store's own recorded state for that cell disagrees with what the worker is
reporting, that disagreement is shown explicitly rather than silently resolved one way
or the other; a worker whose own session has gone stale is never presented as currently
running at all. No active feature, no recorded rationale, no live cells yet, and no
recorded next action each render their own honest line rather than a fabricated
measurement or a hidden field.

### Needs attention

A **generated, severity-ordered list**, not a static panel: independent rules run over
the same data the rest of the board already read, each one firing on its own — no rule
depends on another having fired first — and every item that fires names a suggested
action a human can actually take. Items are ordered heaviest severity first: critical,
then serious, then warning. An empty list says, in one line, that nothing currently
needs attention — it is not a hidden or collapsed section.

The rules, as they exist today:

- **Blocked cells** (critical) — fires when at least one cell needs a human right now;
  names each one. Every blocked cell is treated as its own fix-first item.
- **Unreadable store files** (critical) — fires when any part of the store could not be
  read or parsed; names the file(s), and warns that every other number on the page may
  be incomplete until they are repaired.
- **Work parked, waiting on a person** (critical) — fires when a handoff note is on
  record and it is not explicitly marked as a clean, already-claimed handover to the
  next piece of work; a handoff record with no kind recorded at all is treated the same
  as an explicit pause. Shows when the note was written and its text, and says plainly
  that the store never marks a note as consumed — a stale pause reads as a stale pause,
  dated, never as an invented "probably resolved" judgement.
- **Open P1 review findings** (critical) — fires when a review session that has not yet
  been settled (approved or blocked) carries at least one P1 finding.
- **Unreviewed high-risk work** (serious) — fires when review candidates flagged
  high-risk have never appeared in any review session at all.
- **Gate bypass recorded** (warning) — fires whenever the project's tracked
  configuration records approval gates as being auto-approved at some level; names the
  level, and states explicitly that this is the recorded setting, not necessarily the
  one actually in effect (see "What this board does not claim," below).
- **Knowledge debt** (warning) — fires when the total of (features with capped,
  behavior-changing work that was never folded into the project's own knowledge base)
  plus (capture-queue items still waiting) plus (features with an unresolved
  post-feature proposal note) is greater than zero; the breakdown of that total is
  shown alongside the count.

## Delivery speed

Three headline numbers, shown once at least one feature has shipped:

- **Shipped per working day** — shipped features divided by the number of distinct days
  on which something shipped.
- **Shipped per week** — shipped features across the calendar span from the first ship
  to the last, expressed as a weekly rate. The span counts **calendar dates**, not a
  subtraction of timestamps; subtracting timestamps silently discards a partial day and
  overstates the rate.
- **Typical time to finish** — the median cycle time across shipped features.

**A feature has shipped when every one of its non-dropped cells is capped.** No merge
into a main branch is required, and a dropped cell never blocks shipped status. This
matters because release work, documentation work, and small fixes legitimately land in
the main checkout with no branch at all; requiring a merge marked roughly a third of
real features as never-shipped.

**Cycle time** for a shipped feature runs from its earliest cell claim to its latest
cell cap. A feature missing either timestamp reports no cycle time rather than a
fabricated zero.

A project that has shipped nothing shows an honest statement to that effect, alongside
the list of features still open (any feature with at least one live cell that has not
yet shipped). It never shows zeros dressed as measurements, and no rate ever renders as
a division artifact.

## Work by phase

Every feature still in flight, one card each, grouped into columns by the phase of the
bee lifecycle it is recorded as being in — the phase names themselves are whatever the
store's own records say, never a fixed list this board invents or reorders. Each card
carries the feature's own progress over its live (non-dropped) cells and its recorded
next action, and links onward to that feature's own detail page.

**The feature set placed here is the union of every lane record the store carries and
the one globally active feature the project's own top-level state names** — never the
lane records alone. A project can have an active feature that carries no lane record of
its own; a board that trusted the lane list by itself would silently omit the one
feature actually being worked on. A project with no lane records at all still places
its one active feature correctly. Phase placement is a pure function of the store's own
records — a live worker currently active on a cell never re-places that cell's feature
onto a different phase than the store itself records.

A feature that has fully shipped never appears here — it renders exactly once, down in
Finished, below. A worktree granted to a different feature never contributes its own
cells to this project's phase board, its progress counts, or its shipped status; a
granted worktree's cell ids never appear here at all. A project with nothing tracked by
phase renders one honest line rather than an empty or hidden section.

## Finished

Every feature that has fully shipped, collapsed by default behind one summary line that
states the true count of finished features and finished cells even while the list
itself stays closed — collapsing a list is never allowed to understate what it holds.
Opening it shows one compact line per feature — never one card per cell — naming its
cell count and, when both of its timestamps are on record, its time to finish. This
list is never capped or truncated: every feature that has shipped is named here, no
matter how many there are. A project with nothing finished yet shows a single honest
line instead of a collapsible, zeroed list.

A feature is rendered in exactly one of Work by phase or Finished, never both and never
neither, once it can be placed at all.

## Backlog & Review

Three sub-views in one supporting panel:

- **PBIs by status.** Backlog work items are event-sourced — the same item can appear
  many times as its status changes over time, and its **current** status is whatever
  its most recent recorded entry says. They are grouped and counted by that current
  status, with a bounded, recent slice of individual titles beneath the counts; a
  project with more items than that slice shows the true total alongside the visible
  subset rather than looking smaller than its real backlog. A project with no backlog
  items yet says so plainly.
- **Findings by severity.** Each recorded finding carries a severity of P1, P2 or P3;
  they are summarised by severity, with P1 given visual weight because a P1 finding
  blocks, and the same bounded-recent-slice-with-true-total treatment as PBIs. A
  project with no findings yet says so plainly.
- **Review queue by state.** Every review candidate the store has recorded is placed
  into exactly one of three states by joining it against every recorded review
  session: **unreviewed** (it has never appeared in any session), **in review** (it
  appears in a session whose decision has not yet settled), or **settled** (it appears
  in a session whose decision reached approved or blocked). The count of open P1
  findings is called out first, worded identically to the matching attention-list rule
  so the two surfaces never disagree. Every sentence in this panel words independent
  review as something the project's owner invokes — nothing here ever implies review is
  already running or already queued as pending automatic work.

A store with **zero** recorded review candidates is genuinely ambiguous — it is the
same shape whether a project has never run a review at all, or whether every candidate
has already been folded and rolled off the list — so this one case says review state is
unknown rather than rendering three zeroes as if they were a real measurement. From the
moment even one candidate is recorded, every count is real and computed, including a
genuine zero for "in review" or "settled" when every recorded candidate really is
unreviewed.

## Where work is happening

Three sub-views in one supporting panel, each with its own independent honest-empty
state:

- **Sessions.** Every recorded session, showing where it runs, whether it is currently
  **live** or has gone **stale**, and how long ago it last reported in, in plain
  relative language ("4 minutes ago") — never a raw timestamp. A session's transcript
  path is never shown; it is an absolute path into the user's home directory and has no
  place on an unauthenticated page (see "Renders nothing that identifies a filesystem
  outside the project," below).
- **Worktrees.** Every worktree the project has granted out, shown by its own feature,
  phase, branch and liveness. A worktree whose directory or own state cannot be
  resolved is marked plainly unresolved rather than being silently dropped from the
  list.
- **Workspaces.** Every workspace record the project's own store knows about, plain.

## Process health

Three or four sub-views in one supporting panel:

- **File-lock contention** — every reservation currently held and not yet released,
  naming the path, the agent and the cell involved. A released reservation is history,
  not contention, and is left out. No contention right now renders one honest line.
- **Model-tier mix** — one count per tier value the store actually used (never limited
  to a fixed list of expected tiers, so an unrecognised value still shows rather than
  vanishing), plus the share of tiered cells sitting on the single most expensive tier.
  No cells to measure, and a store where every cell is untiered, each render their own
  honest line rather than a fabricated percentage.
- **Gate bypass** — the project's own tracked bypass setting, worded identically to the
  matching attention-list rule when it is not off, and a plain statement when it is
  recorded as off. A project with no tracked configuration file at all, or one that
  failed to parse, is a distinct "unknown" state, never rendered the same as "off" —
  off is something the file must have positively recorded.
- When any part of the store could not be read, that list of unreadable files also
  appears here — a partly-unreadable store is a process-health signal in its own right,
  not a separate footer bolted onto the page.

## Drilling in

Every cell on the board links to its own page, and every feature name links to its own
page.

- A **cell page** shows that cell in full: what it is, what proves it, its state and
  lane, the files it touches, the decisions it cites, its required outcomes, and its
  whole execution trace — who ran it, when it was claimed and capped, its outcome,
  recorded deviations, and its test result. The list of files a cell touches lives only
  here — the board's own cards never carry it, keeping the board itself scannable.
- A **feature page** shows whether the feature shipped, its cycle time, and all of its
  cells grouped into the same four cell-state buckets the board's headline numbers
  count, each linking onward to its cell page.

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
fields the board renders are free-form prose from the store — a recorded next action, a
routing rationale, a handoff note, a review finding's own description — and any of
these can have an absolute path typed into the middle of a sentence, not as the whole
field. Every one of those fields is scanned for an absolute path embedded anywhere
inside it, and any path found is reduced the same way a wholly-path field would be,
while the words around it survive untouched. A project's operator can write "see
/home/them/notes.txt for context" into a next action and that sentence still renders,
with the path portion alone reduced.

A feature name is itself free text from the store, and this board occasionally has to
join a feature name onto a location on disk — today, only to check whether a feature
has an unresolved post-feature note waiting. Before any such join happens, the name is
validated: no path separators, no `..` segment, no leading dot, no control characters,
and not already an absolute path of any shape. A name that fails validation is never
looked up at all — the join is never attempted, so a maliciously- or accidentally-
shaped feature name can never make this board read, or claim to check, anything outside
the project it was asked about.

The tests that guard both halves of this guarantee assert against the **fixture's own
root path**, not against a literal that merely looks like a production path — a check
written against one hardcoded prefix would pass green while a real page leaked a real
path verbatim. See `docs/history/learnings/20260805-toothless-security-assertions.md`.

## What this board does not claim

The gate-bypass value shown anywhere on this board — in the attention list and in
process health — is the value the project's own **tracked** configuration file
records, exactly as written. It is not necessarily the **effective** value: bee
supports a separate, machine-local configuration overlay that is never checked into the
project and is never read by this board. A project's tracked configuration can record
bypass as off while a particular machine's own overlay actually has it on, or vice
versa. This board makes no claim about that overlay at all — it says what the tracked
file says, and labels it as such, rather than attempting to resolve the effective value
and risk being confidently wrong on the one machine whose overlay disagrees.

## Honesty rules that hold everywhere

Four rules apply across every section of this board, not just the ones above that
happen to illustrate them:

- **A dropped cell counts toward no total and no denominator, anywhere.** Not in the
  headline numbers, not in a phase card's progress, not in whether a feature counts as
  shipped. It never shipped, so counting it as done would inflate the picture; it is
  simply absent from every count that would otherwise include it.
- **A capped or truncated list always states its true total beside the visible
  subset.** A real store can be large — hundreds of backlog rows and findings are
  normal — so detail lists are bounded to a recent slice, and whenever that slice is
  smaller than the true total, the panel says so. The one list on this board that is
  never capped is the finished-features list — nothing that has shipped is ever
  silently left off it.
- **Nothing to measure renders as a stated absence, never as a zero or a division
  artifact.** "No features have shipped yet," "no live cells recorded for this feature
  yet," a stat tile showing a plain dash instead of a fabricated `0.0` — these are the
  shape this rule takes. A number that is genuinely, computably zero — a bucket that
  really does hold no cells right now, or a feature whose real, measured cycle time
  happens to round to a very small figure — is not what this rule forbids; it forbids
  manufacturing a number where there was no measurement to take.
- **A store that cannot be fully read says so.** Any single unreadable file — missing,
  empty, truncated, or malformed — degrades the page to a partial view that names what
  could not be read, both in the needs-attention list and in process health. It never
  takes down the page, and a malformed line among otherwise-good lines loses only
  itself. A project with no store at all is a different, earlier case — presence, not
  degradation — and is a clean not-found, never an empty dashboard (see "Where it
  appears," above).

## Independent review is always invoked

Wherever this board mentions independent review — the lifecycle stepper, the
needs-attention rules that reference it, the review queue panel — it is worded the same
way: review is something the project's owner invokes, never a stage the board implies
is already running, already queued, or pending on its own. A gate that has not yet been
through review reads as "not yet approved," never as "review in progress"; a candidate
that has never appeared in a session reads as "never reviewed," never as "awaiting
automatic review." This holds even when the count of unreviewed or high-risk work is
large enough that a different phrasing might read as more urgent — the wording never
implies the board itself is doing, or about to do, that work.

## Bounded output

The snapshot is rebuilt on every request, and a real store is large — hundreds of
backlog rows and thousands of decision events are normal. Detail lists are capped at a
small recent slice, and each panel states its true total when it is showing a capped
subset (see "Honesty rules that hold everywhere," above).

Only live cells are read. The archive that `bee close` produces is not consulted; at
the time this was decided, live cells outnumbered archived ones roughly forty to one.
This is worth revisiting if archiving becomes routine.

## Scope

This surface covers **one project at a time**. A single page aggregating every
registered project — total active projects, velocity across all of them, which lanes
run where across the fleet — is a separate, later feature.
