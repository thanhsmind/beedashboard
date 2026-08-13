promote proposal for work item "upstream-code-viewer" (.bee/logs/scribing-runs.jsonl) — 1 capped cell(s): upstream-code-viewer-1
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/upstream-code-viewer/delivery.md

---
type: bee.delivery
title: upstream-code-viewer — delivery
description: "Delivery record proposed by bee knowledge promote for work item upstream-code-viewer: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: upstream-code-viewer-delivery
  lifecycle: active
  areas: [web-interface, system-overview]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/archive/upstream-code-viewer/upstream-code-viewer-1.json]
---

# upstream-code-viewer — Delivery

## What shipped

- **upstream-code-viewer-1** — Code viewer ported from upstream across twelve commits; topbar hand-merged to keep this fork's brand and menu; upstream's auth-dependent tests adapted (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **upstream-code-viewer-1** — `cargo test --workspace green at 886, up from 878 before the block. Two upstream tests that depend on upstream authentication are adapted: the login-token test is removed with its reason recorded in place, and the cookie argument is dropped from every http_get in the new Code tests. Live check after install: /p/<id>/_code/ and a source file both answer 200, the sidebar lists folders before files, the breadcrumb right half reads the file's language and size, and the brand still reads Bee Artifact.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work upstream-code-viewer` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "upstream-code-viewer" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-13T07:20:28.110Z), the work item declares no bee.areas.

area web-interface:
  - [upstream-code-viewer-1] Code viewer ported from upstream across twelve commits; topbar hand-merged to keep this fork's brand and menu; upstream's auth-dependent tests adapted — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/archive/upstream-code-viewer/upstream-code-viewer-1.json)

area system-overview:
  - [upstream-code-viewer-1] Code viewer ported from upstream across twelve commits; topbar hand-merged to keep this fork's brand and menu; upstream's auth-dependent tests adapted — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/archive/upstream-code-viewer/upstream-code-viewer-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 2 area bullet(s), 0 pattern candidate(s), 0 file(s) written.