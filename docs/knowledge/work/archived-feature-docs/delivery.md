---
type: bee.delivery
title: archived-feature-docs — delivery
description: "Delivery record for work item archived-feature-docs: 1 capped cell(s), 1 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: archived-feature-docs-delivery
  lifecycle: active
  areas: [bee-cockpit]
  required_context: [docs/specs/bee-cockpit.md]
  sources: [.bee/cells/archived-feature-docs-1.json]
---

# archived-feature-docs — Delivery

## What shipped

A finished piece of work keeps its record after its tasks are filed away. The
cockpit, however, only knew about a piece of work while at least one of its
tasks was still in the live pile. Once the last one was filed, the work
vanished from the roster the cockpit builds each time it reads the project —
and every page that asks "what is this work about?" asks that roster first.

The visible damage was on the page for the work itself: it still rendered, but
with no documents row, no title and no description, because the reader that
fetches those keys off the roster and found nothing there.

The roster now counts filed-away work as well as live work. A finished piece of
work carries its title, its description, its documents row and its knowledge
proposal exactly as it did the day before its last task was filed.

## Verify

`cargo test --workspace` green, with the failing case written first: a project
whose only work is filed away now appears in the roster.

## Deviations

Ran in the main checkout rather than a branch checkout of its own — a solo
one-file fix with no other session live, which the working agreement allows.

## Provenance

Written from the capped cell trace of `archived-feature-docs-1` and the capture
stub it left behind. The rule it settled — every per-work reader keys off the
same roster, so the roster must count filed-away work — is recorded in the
capture queue.
