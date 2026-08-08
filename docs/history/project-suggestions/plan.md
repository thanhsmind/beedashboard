---
artifact_contract: bee-plan/v1
mode: high-risk
# approved_gate2: <unset>
---

# Plan: Project Suggestions From Running Sessions

Mode: `high-risk` — 1 risk flag: audit-security, which is a hard gate on its
own. This plan is deliberately short for the lane: the change is one computed
list and one block of markup, and what earns the lane is a single question —
what a page with no authentication is now allowed to say about this machine.

## Requirements (locked decisions)

- **S1** — suggestions come from herdr: every session whose working directory
  sits under no registered project. Not a filesystem scan.
- **S2** — a suggestion points at the pane's own working directory, exactly as
  herdr reports it. No walk up to a repository root.
- **S3** — supersedes web-interface R5 in part: the suggestion block shows full
  filesystem paths. Rows for already-registered projects still never do.

## Discovery

- `unassigned_panes` (`crates/mdview/src/server.rs:1861`) already computes the
  complement of every project's boundary — exactly S1's set. Two facts about
  it matter here: it iterates `snapshot.agents`, not `snapshot.panes`, so a
  plain shell in an unregistered folder is invisible to it; and it fails
  closed to an empty list when any registered project's root cannot construct
  a `Boundary` (`server.rs:1867-1877`).
- Its cwd comes straight off the pane, unvalidated and possibly empty
  (`server.rs:1884-1889`) — the doc at `server.rs:1855-1860` says so on
  purpose: there is no containment claim to make for a pane that belongs to no
  project.
- Every existing caller of `unassigned_panes` checks `unassigned_group_enabled`
  as well as `terminal_family_enabled` — the group's own switch, added by
  toa-4/D9 precisely because naming host-wide panes on an open page is a
  disclosure. The block this plan adds reads the same set.
- `index_page` (`server.rs:302`) already takes one timeout-wrapped snapshot
  behind `terminal_family_enabled`, so the data is in hand with no second call.
- `POST /api/projects/register` (`server.rs:197`) already carries the whole
  D9a/D9b guard chain. A suggestion's button posts to it and inherits every
  refusal; nothing new validates a path.
- Evidence command: `cargo test --workspace`.

## The question this lane exists for

S3 settles that a suggestion may print a path. It does not settle **which
switch** governs the block. Two readings, and they differ in what a stranger
who reaches the port sees by default:

- Gate on `terminal.enabled` alone — the block appears wherever badges do.
  Consistent with the user's rejection of "hide it behind its own switch", and
  the feature is visible the moment it ships.
- Gate on `terminal.enabled` **and** `unassigned_enabled` — the precedent every
  other reader of this exact set follows. Off by default, so the feature is
  invisible until switched on, which is what the user declined.

Recommendation: `terminal.enabled` alone, recorded as narrowing toa-4/D9's
scope for this one surface, because the user was shown the switch option and
chose against it. The consequence, stated plainly: with the terminal switch on,
anyone who can reach the port learns the full path of every folder on this
machine where a coding agent is running outside a registered project. That is
strictly more than the Unassigned card discloses today, which is only that the
group exists.

## Approach

Add `suggested_projects(snapshot, projects) -> Vec<ProjectSuggestion>` beside
`unassigned_panes`, iterating `snapshot.panes` rather than `snapshot.agents`
so an unregistered folder holding only a shell is suggested too, deduplicating
by directory and carrying a session count. Keep its fail-closed behaviour
identical: one unconstructable project root empties the whole list, for the
same reason the existing function gives — without a working boundary there is
no way to tell that project's own panes from a genuinely unassigned one, and
guessing would leak them.

Drop a pane whose cwd is empty, and drop any directory that is already a
registered project's root (a pane can sit in a registered project's parent).
Render the block under the project list: one row per directory, its basename
as the name, its full path, the session count, and a form posting the path to
the existing register route. Refusals come back through that route's own fixed
codes — a suggestion that turns out to be deny-listed or oversized is refused
exactly like a hand-typed path.

Rejected: a new register endpoint for suggestions (the existing one's guards
are the point); dismissing or remembering suggestions (no decision asks for
it, and it needs storage); walking up to a git root (S2 rules it out).

Risk map:

| Component | Risk | Reason | Proof needed |
|---|---|---|---|
| The block's gate | HIGH | Decides what an unauthenticated page says about this machine by default | Route test: switch off ⇒ no path, no block, no herdr call |
| Set computation | MEDIUM | A wrong complement prints a registered project's own folders as "unregistered" | Route test: a pane inside a registered project never appears; one outside does |
| Fail-closed | MEDIUM | An unconstructable root must empty the list, not leak | Route test mirroring the existing unassigned fail-closed case |
| Register button | LOW | Reuses the guarded route | Route test: posting a suggestion registers it and the row moves into the list |

## Test matrix

| Dimension | Probe |
|---|---|
| Exposure | `terminal.enabled` off ⇒ `/` contains no suggestion block and no filesystem path |
| Partition | A pane under a registered project is never suggested; a pane outside one is |
| Shells | A pane with no agent in an unregistered folder is still suggested (the existing function would miss it) |
| Dedup | Two sessions in one folder produce one suggestion carrying a count of two |
| Empty cwd | A pane herdr reports with no working directory is dropped, not suggested as an empty path |
| Already registered | A directory that is exactly a registered project's root never appears |
| Fail-closed | One project whose root cannot construct a `Boundary` ⇒ the block is empty, not populated |
| End to end | Posting a suggestion's path registers it, and it appears as a row on the next load |
| Refusal parity | A suggestion whose path is deny-listed is refused by the register route's own code, not by a second rule |

## Out of scope

- Dismissing, hiding or remembering individual suggestions.
- Any filesystem scan for candidate projects — S1 rules it out.
- Changing the Unassigned group, its switch, or its markup.
