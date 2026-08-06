promote proposal for work item "daemon-lock-stale" (.bee/logs/scribing-runs.jsonl) — 1 capped cell(s): dls-1
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/daemon-lock-stale/delivery.md

---
type: bee.delivery
title: daemon-lock-stale — delivery
description: "Delivery record proposed by bee knowledge promote for work item daemon-lock-stale: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-06
bee:
  id: daemon-lock-stale-delivery
  lifecycle: active
  areas: [daemon]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/dls-1.json]
---

# daemon-lock-stale — Delivery

## What shipped

- **dls-1** — Reproduced the hang: health_check's TcpStream::connect had no connect timeout, so a dead-port connect blocks indefinitely on a network that drops rather than refuses -- fixed with connect_timeout(500ms); also fixed stop_daemon killing by pid unconditionally (a recycled pid risk) by checking health_check before kill (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **dls-1** — `cargo test --workspace. The reproduction is the proof: a test that FAILS before the fix and passes after, driving the stop path against a stale lock naming a dead pid and a dead port, asserting it returns within a bounded time and clears the lock. Plus: (edge) a lock naming a pid that is alive but is not mdview — the process must not be killed and the lock must not be cleared blindly; (edge) no lock file at all still reports no daemon running; (regression) a genuinely live daemon is still stopped and its lock cleared; (regression) the orphaned-daemon guard at cli.rs:423 still refuses to clear the lock of a daemon that failed to die but still answers. State in the outcome what the root cause actually was, and say so plainly if it turned out not to be a hang at all.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work daemon-lock-stale` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "daemon-lock-stale" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-06T22:01:45.954Z), the work item declares no bee.areas.

area daemon:
  - [dls-1] Reproduced the hang: health_check's TcpStream::connect had no connect timeout, so a dead-port connect blocks indefinitely on a network that drops rather than refuses -- fixed with connect_timeout(500ms); also fixed stop_daemon killing by pid unconditionally (a recycled pid risk) by checking health_check before kill — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/dls-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.