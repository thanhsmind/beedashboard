---
type: bee.delivery
title: projects-tab-ordering — delivery
description: "Projects tab drops its page heading and floats agent-active project groups above idle ones."
timestamp: 2026-08-16
bee:
  id: projects-tab-ordering-delivery
  lifecycle: active
  required_context: []
  sources: [crates/waggledance/src/views.rs]
---

# projects-tab-ordering — Delivery

## What shipped

The Projects tab page heading was removed, and project groups with agent
activity — any non-shell pane, same rule the badge filter uses — sort above
idle groups in the project list (`project_list_main`).

## Verify

`cargo test` green: `project_list_drops_heading_and_floats_agent_active_projects_first`.
Deployed to artifact.gogl.be the same day.

## Provenance

Flushed 2026-08-16 from capture stub afd67f3f (settlement logged in the
decision log the turn it settled; area tag `home-projects`).
