---
type: bee.delivery
title: card-terminals — delivery
description: "Delivery record for work item card-terminals: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: card-terminals-delivery
  lifecycle: active
  areas: [bee-cockpit]
  required_context: [docs/specs/bee-cockpit.md]
  sources: [.bee/cells/archive/card-terminals/card-terminals-1.json]
---

# card-terminals — Delivery

## What shipped

The cross-project Features board's Waiting on you and In Progress cards now
carry one marker per terminal session running in that feature's own checkout,
each opening that session's own terminal view.

Nothing in the record ties a session to a feature — a session knows which
feature it works but not which terminal it occupies, and a terminal knows its
directory but not its feature — so the marking is decided by the checkout. For
a feature with its own branch checkout that is exact; for one working in the
main checkout the markers are shared with every other main-checkout feature,
and are labelled as the terminals of *this checkout* rather than the feature's
own.

## Verify

`cargo test --workspace` green at 843, up from 837. Six new tests: two on the
card's own markup and its empty case, four through the router covering
worktree-versus-main scoping, a feature with no session, the terminal switch
off, and the Finished rows carrying none. The twelve existing home-page tests
stayed green and unedited.

## Deviations

None recorded in the capped cell trace.

## Provenance

`bee knowledge promote` proposed area-update bullets for this work item. They
were reviewed and not applied: each restated the cell's outcome in code terms —
function and file names — where an area spec takes business language only, and
the behaviour itself was already merged into the touched specs by hand. The
reason is recorded in the decision log.
