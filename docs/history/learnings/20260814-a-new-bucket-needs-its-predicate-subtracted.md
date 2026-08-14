---
date: 2026-08-14
feature: kanban-columns
categories: [pattern, failure]
severity: standard
tags: [classification, planning, review, verify]
---

# Learning: A new bucket needs its predicate subtracted from the incumbent

**Category:** pattern
**Severity:** standard
**Tags:** [classification, planning]
**Applicable-when:** adding a case to any ordered classification chain — a
column, a status, a route, a dispatch table — where an existing case already
matches broadly.

## What Happened

The plan added a Todo column to the feature board, defined as "open cells with
none claimed", placed last in the chain behind In Progress. In Progress tested
`has_live_work`, which counted `waiting` (open) cells among live work. Every
feature Todo was meant to hold therefore satisfied In Progress first, and the
new branch could never fire. The plan's own happy-path test — "a feature with
only open cells lands in Todo" — contradicted the plan's own chain. An
independent plan review caught it before any code was written; the fix was a
decision (D10) narrowing live work to `doing` and `stuck` cells, not a
reordering.

## Root Cause

The incumbent branch's predicate was broader than its name suggested. Reading
"In Progress" as a label rather than as its actual boolean made the overlap
invisible, and appending a branch to the end of a chain feels additive when it
is really a subtraction problem: the new case's population has to be carved out
of an existing case's population, or the new case is dead.

## Recommendation

When adding a case to an ordered classification chain, write out the new case's
predicate and every earlier predicate as booleans over the same inputs, and name
one concrete instance the new case must catch. Trace that instance through the
chain from the top. If an earlier branch claims it, the change is not "add a
branch" — it is "narrow an existing branch and add a branch", and the narrowing
is its own decision worth locking.

---

# Learning: A cap without a compile lands red on main

**Category:** failure
**Severity:** standard
**Tags:** [verify, cells]
**Applicable-when:** a cell records a verify owner other than the worker.

## What Happened

Cell `homepage-terminals-1` was capped carrying
`verify_owner: "main (feature close) — the worker never runs this"`. Its commit
landed on main referencing a type and a function that were never written and a
call site left at six of nine arguments, so `cargo test --workspace` did not
compile. The next feature discovered it only when its own base check ran, hours
later, and lost a full turn diagnosing whose code was broken and whether it was
uncommitted work in flight.

## Root Cause

Deferring verification to a later close means nothing between the cap and that
close is standing on proof. The cap read as done to every reader — the cell
status, the commit, the board — while the build was broken.

## Recommendation

Never cap a cell whose verify the worker did not run. If a cell genuinely cannot
run the declared suite, it is blocked, not capped: record it as blocked with the
reason. Before a first claim in any feature, run the declared test command and
treat a red as its own fix-first cell rather than as background noise — that
check is what caught this one.
