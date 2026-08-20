---
type: bee.pattern
title: A cell that promises a user-visible outcome owes one proof of the whole path
description: "Pitfall: slicing a user-visible outcome into per-unit cells lets every slice go green while the feature stays inert, because no proof ever crosses the seams between them."
timestamp: 2026-08-20
bee:
  id: prove-the-whole-path
  lifecycle: active
  areas: [notifications, orchestration]
  sources: [.bee/cells/archive/dispatch-blocked-notify/dbn-3.json, .bee/cells/archive/dispatch-blocked-notify/dbn-4.json, docs/knowledge/work/dispatch-blocked-notify/delivery.md]
  polarity: pitfall
---

# A cell that promises a user-visible outcome owes one proof of the whole path

## The trap

A feature the user can name — "the human gets told when a worker is stuck" —
gets sliced into units: the store, the raise, the switch, the suppression.
Each unit is small, each gets its own tests, each goes green. The seams
between them belong to no unit, so nothing tests them, and the feature can
ship dead while every proof in it is honest.

## What it looked like here

Three green cells produced an inert feature. One added storage that dedupes
alerts. One raised an alert at the right moment. One armed a switch and
exposed the store behind it. Every test passed. Nothing reached a human: the
arming happened in the long-running service while the raising happened in a
separate short-lived process, and no store could ever travel between them.
The gap was found by reading who calls the accessor — nobody outside its own
tests — not by any test failing. It cost an extra cell to repair.

## The tell

Look for a proof set whose members are each scoped to one file or one module,
covering a feature whose promise is stated in the user's words. If no single
proof runs from the trigger the user causes to the effect the user sees, the
seams are unproven no matter how many tests are green. A second tell: an
accessor, hook, or setter with no production caller — a link built toward a
consumer that never arrived.

## What to do instead

- When a cell's stated truth is user-visible, one of its proofs must exercise
  the whole path, even if it must fake the ends. Per-unit tests stay; they
  just stop being the only evidence.
- Before accepting a slice as done, ask who calls the thing it added. A
  production caller is the answer; "the tests" is the warning.
- Name the process, service, or boundary each side of a link lives in while
  the work is being shaped. Two things in one repository are not two things in
  one process, and a plan that never says so lets a worker assume they are.
