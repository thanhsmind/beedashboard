promote proposal for work item "term-url-links" (.bee/lanes/term-url-links.json) — 3 capped cell(s): term-url-links-1, term-url-links-2, term-url-links-3
anchor: ledger — .bee/lanes/term-url-links.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/term-url-links/delivery.md

---
type: bee.delivery
title: term-url-links — delivery
description: "Delivery record proposed by bee knowledge promote for work item term-url-links: 3 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-14
bee:
  id: term-url-links-delivery
  lifecycle: active
  required_context: [.bee/lanes/term-url-links.json]
  sources: [.bee/lanes/term-url-links.json, .bee/cells/term-url-links-1.json, .bee/cells/term-url-links-2.json, .bee/cells/term-url-links-3.json]
---

# term-url-links — Delivery

## What shipped

- **term-url-links-1** — Added linkify_urls beside linkify_docs, wired into both terminal screen routes and styled .term-url-link (3 file(s) changed)
- **term-url-links-2** — A bare http:// or https:// with no host behind it stays plain text; only a scheme followed by a host character links. (1 file(s) changed)
- **term-url-links-3** — A URL stops at any HTML entity except &amp;, so a quoted URL links without the closing entity in its href. (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **term-url-links-1** — `cargo test --workspace`
- **term-url-links-2** — `cargo test --workspace`
- **term-url-links-3** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work term-url-links` from 3 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/lanes/term-url-links.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 3 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.