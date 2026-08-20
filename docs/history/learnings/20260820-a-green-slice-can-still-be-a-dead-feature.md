# Learning: A green slice can still be a dead feature

**Category:** failure
**Severity:** standard
**Tags:** [planning, tests, verify, workers, delegation]
**Applicable-when:** a user-visible outcome is sliced into per-unit cells, or work is handed to external CLI workers that cannot see the whole system.

## What Happened

dispatch-blocked-notify shipped in five cells. Three of them — storage,
raise point, switch wiring — went green with their own tests and left the
feature inert: the switch was armed inside the long-running service while
the alert was raised inside the separate MCP process, so the store could
never travel between them. No test failed. The gap surfaced only when the
orchestrator grepped for callers of the accessor the third cell added and
found none outside its own test module. Repairing it cost a fourth cell.

Two smaller misses rode along. A worker capped a cell after running only its
scoped test, leaving five formatting violations that would have gone red in
CI; adding one line to the next brief ("run cargo fmt --all --check before
you cap") made the next cell come back clean. And the plan's own test matrix
listed a happy path, edge cases and error paths per unit — never one proof
crossing the units — so the plan itself authorised the hole.

## Why It Happened

Slicing follows the code's structure; proofs then follow the slices. The
seams between slices belong to no cell, so no cell owes them evidence. An
external worker makes this sharper: it sees only its brief, so it cannot
notice that the thing it is wiring to lives in another process, and it will
happily satisfy the letter of its cell.

## What To Do Differently

- When a cell's stated truth is user-visible, one proof must run the whole
  path — the per-unit tests stay, they just stop being the only evidence.
- Before accepting a slice, ask who calls what it added. "The tests" is the
  warning sign; a production caller is the answer.
- Name the process or service boundary each side of a link sits in during
  shaping. Two modules in one repository are not two modules in one process,
  and a brief that never says so lets the worker assume they are.
- Put the project's full check, not just the scoped test, into the worker's
  brief. A worker proves what it is asked to prove and nothing more.

## Promoted

`docs/knowledge/patterns/prove-the-whole-path.md` — the pitfall, its tells,
and what to do instead.
