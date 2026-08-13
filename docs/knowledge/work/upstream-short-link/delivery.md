---
type: bee.delivery
title: upstream-short-link — delivery
description: "Delivery record for work item upstream-short-link: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: upstream-short-link-delivery
  lifecycle: active
  areas: [system-overview, web-interface]
  required_context: [docs/specs/bee-cockpit.md]
  sources: [.bee/cells/archive/upstream-short-link/upstream-short-link-1.json]
---

# upstream-short-link — Delivery

## What shipped

Every indexed file now also answers at a short, opaque address of its own,
alongside its full path-shaped URL. The agent tool hands out the short one and
names the file's path beside it in plain text, since a transcript of opaque
codes says nothing about which file each one was.

Ported from upstream `vantt/mdview` (commit 7518cd0), the first of that line of
development brought across after the two repositories diverged on 2026-07-20.

## Verify

`cargo test --workspace` green at 867, up from 844 — the rise is upstream's own
short-link tests arriving with the commit. Three conflicts resolved: the module
list took both sides; the daemon health check kept this fork's connect timeout
while adopting upstream's split of that function, so its early exits answer the
new return shape; and one upstream test gained the version field this fork
records in its lock file. No pre-existing test was edited beyond the
initializer that would not compile.

## Deviations

None recorded in the capped cell trace.

## Provenance

`bee knowledge promote` proposed area-update bullets for this work item. They
were reviewed and not applied: each restated the cell's outcome in code terms —
function and file names — where an area spec takes business language only, and
the behaviour itself was already merged into the touched specs by hand. The
reason is recorded in the decision log.
