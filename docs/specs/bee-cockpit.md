# Bee Cockpit

A read-only surface inside mdview that shows what the bee harness is doing in a
registered project: the work in flight, how fast features ship, the backlog, the
running sessions, and the detail behind any of it.

Technology-agnostic: this describes behavior and rules, not the Rust that implements
them. Code entry points are listed in `reading-map.md`.

## Who it is for

Someone running bee across several projects who cannot read bee's own store. bee keeps
a thorough record — cells, lanes, sessions, backlog, decisions — spread across JSON and
JSONL files in each project's `.bee/` directory. That record is precise and
unreadable. This surface answers, in plain language, the questions a human actually
asks: what is being worked on, what is stuck, how fast are we shipping, and what is
behind that number.

## Where it appears

A project qualifies for the bee surface when **both** hold:

1. It is registered with mdview (it appears in the project registry).
2. Its root directory contains a `.bee/` directory.

A registered project without `.bee/` behaves exactly as it always did — opening it
still goes straight to its entry document. No bee link appears, and requesting a bee
page for it returns not-found rather than an empty bee page. The absence of a store is
never rendered as an empty dashboard.

A qualifying project gains an entry point on its home page leading to its board.

## Read-only, always

The surface never writes to a project's `.bee/` directory. It approves no gate, claims
no cell, edits no backlog item, and ends no session. Those actions belong to the bee
CLI and to the live sessions that own that state; a dashboard that wrote there would
race a running agent.

This is enforced, not merely intended: the test suite snapshots a fixture's entire
`.bee/` tree before a request and asserts it is byte-identical afterwards. A caching
layer that wrote into a project's own store would fail that test.

## What the board shows

### Work in flight — four buckets

bee records five cell states. They map to four things a human needs to distinguish:

| Bucket | Underlying state | Meaning |
|---|---|---|
| **Doing** | claimed | Someone or something is working on it now |
| **Waiting** | open | Ready, nobody has taken it |
| **Stuck** | blocked | Needs a human; rendered as its own red state |
| **Done** | capped | Finished, tests green |

A **dropped** cell appears in no bucket and counts toward nothing — it never shipped,
so counting it as Done would inflate the picture. Stuck is deliberately its own bucket
rather than folded into Waiting: stuck work is the thing most worth seeing at a glance,
and it must never hide inside a queue.

A cell carrying a state this surface does not recognise is counted in no bucket and
does not break the page.

### Ship velocity — three headline numbers

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

A project that has shipped nothing shows an honest statement to that effect. It never
shows zeros dressed as measurements, and no rate ever renders as a division artifact.

### Backlog

Two kinds of row share bee's backlog file and are presented separately:

- **Work items**, which are event-sourced: the same item appears many times as its
  status changes, and the **last** entry is its current status. They are shown grouped
  by that current status.
- **Findings**, each carrying a severity of P1, P2 or P3. They are summarised by
  severity, with P1 given visual weight because P1 blocks.

### Sessions

Each session shows where it runs, whether it is **live** or **stale**, and how long ago
it last reported in — in plain relative language ("4 minutes ago"), never a raw
timestamp. A session counts as live when it reported within the last 30 minutes.

A session's transcript path is **never** shown. It is an absolute path into the user's
home directory and has no place on an unauthenticated page.

### Lanes and workspaces

The lane records and worktree workspaces, with their branches, so a reader can see
which feature is running where.

## Drilling in

Every cell on the board links to its own page, and every feature name links to its own
page.

- A **cell page** shows that cell in full: what it is, what proves it, its state and
  lane, the files it touches, the decisions it cites, its required outcomes, and its
  whole execution trace — who ran it, when it was claimed and capped, its outcome,
  recorded deviations, and its test result.
- A **feature page** shows whether the feature shipped, its cycle time, and all of its
  cells grouped into the four buckets, each linking onward to its cell page.

An unknown cell or feature name returns a clean not-found, never a blank page.

## Two rules that shape everything

### No absolute filesystem paths, anywhere

This surface carries no authentication of its own — like every mdview route
outside the agent terminal family (see the Agent terminal spec) — and can be
bound to a non-loopback address. bee's store is full of absolute paths — the
files a cell touches, a worker's identity, a session's transcript, a
workspace root. **None of them may reach the page.** Path-shaped values are
rendered relative to the project root or dropped.

The tests that guard this assert against the **fixture's own root path** and against
absolute-path shape in general. They deliberately do not assert on a production-looking
literal: fixtures build under the system temp directory, so a check for one specific
home-directory prefix passes green while the page leaks paths verbatim. See
`docs/history/learnings/20260805-toothless-security-assertions.md`.

### Bounded output

The snapshot is rebuilt on every request, and a real store is large — hundreds of
backlog rows and thousands of decision events are normal. Detail lists are capped at a
small recent slice, and **each panel states its true total when it is showing a
capped subset**, so nothing on the page looks smaller than it is.

Only live cells are read. The archive that `bee close` produces is not consulted; at the
time this was decided, live cells outnumbered archived ones roughly forty to one. This
is worth revisiting if archiving becomes routine.

## Degrading honestly

Any single unreadable file — missing, empty, truncated, or malformed — degrades to a
partial view that names what could not be read. It never takes down the page. A
malformed line among good lines loses only itself.

## Scope

This surface covers **one project at a time**. A single page aggregating every
registered project — total active projects, velocity across all of them, which lanes
run where across the fleet — is a separate, later feature.
