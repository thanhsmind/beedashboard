---
type: bee.delivery
title: upstream-code-viewer — delivery
description: "Delivery record for work item upstream-code-viewer: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: upstream-code-viewer-delivery
  lifecycle: active
  areas: [web-interface, system-overview]
  required_context: [docs/specs/web-interface.md, docs/specs/system-overview.md]
  sources: [.bee/cells/archive/upstream-code-viewer/upstream-code-viewer-1.json]
---

# upstream-code-viewer — Delivery

## What shipped

The viewer could show a project's written documents and nothing else. Reading
the code those documents describe meant leaving for an editor, which broke the
one thing the viewer is for: following a thread through a project without
switching tools.

The upstream project had already built a code browser. It was brought across
whole rather than rebuilt: browsing a repository's tree, opening a file with
its syntax coloured, and reaching it by address like any other page.

Two things were deliberately not taken. The brand and menu of the top bar
stayed this fork's own, merged by hand where the incoming version would have
overwritten them. And the incoming tests that assumed an account system this
fork does not have were adapted to run without one, rather than dropped.

## Verify

`cargo test --workspace` green, including the end-to-end open path.

## Deviations

None recorded.

## Provenance

Written from the capped cell trace of `upstream-code-viewer-1` and the promote
proposal at `docs/history/upstream-code-viewer/promote-proposals.md`. The port
predates the project rename, so the trace names the old crate paths.
