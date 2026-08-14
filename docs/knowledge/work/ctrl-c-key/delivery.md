---
type: bee.delivery
title: ctrl-c-key — delivery
description: "Delivery record for work item ctrl-c-key: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: ctrl-c-key-delivery
  lifecycle: active
  areas: [agent-terminal, web-interface]
  required_context: [docs/specs/agent-terminal.md, docs/specs/web-interface.md]
  sources: [.bee/cells/ctrl-c-key-1.json]
---

# ctrl-c-key — Delivery

## What shipped

Watching a terminal from the browser meant watching a runaway command finish on
its own: the key row offered movement and Tab, but no way to interrupt. The one
key a person reaches for when something will not stop was missing.

The key row now ends with an interrupt button, sending the same signal a
keyboard would, under the name the terminal service accepts on the wire.

## Verify

`cargo test --workspace` green.

## Deviations

None recorded.

## Provenance

Written from the capped cell trace of `ctrl-c-key-1`. The work predates the
project rename, so its trace names the old crate path.
