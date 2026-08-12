# A process that only watches forward is blind to everything before it started

**Date:** 2026-08-12
**Found in:** stale-index-refresh — a 404 on a file the reader could see in their editor
**Applies to:** any long-running process that maintains a cache, index, or
mirror of state it does not own

## The bug, stated plainly

`mdview serve` spawned a filesystem watcher and began serving. The watcher is
correct: every create, edit and delete from that instant forward reached the
index. What no code covered was the interval *before* that instant. A file
committed while the daemon was down produced no event, the lookup behind
`/p/<id>/<path>` is a `rel_path` row match in sqlite rather than a filesystem
probe, and so the page answered 404 about a file plainly present on disk — and
kept answering it, indefinitely, until a human ran `mdview refresh` by hand.

The gap had existed since the watcher was written. It surfaced only because a
reader had no terminal at hand and asked the obvious question: why does this say
not found?

## The general shape

An incremental updater is a **delta** mechanism. It is only ever correct
relative to a base that something else must establish. Wire up the deltas
without the base and the system is exactly as correct as the assumption "nothing
changed while I was not looking" — an assumption that is false on every restart,
every crash, every deploy, and every `git checkout` performed with the process
down.

Before shipping a watcher, a subscription, a tail, or a replication stream, ask
the two questions the delta cannot answer for itself:

1. **What establishes the base?** Something must reconcile against the source of
   truth at least once at startup. In this repo that door already existed and
   was already tested — `Engine::refresh`, the same call `mdview refresh` makes.
   The fix was to walk through it on boot, not to write new indexing logic.
2. **What does a reader do when the base is wrong anyway?** Reconciliation can
   only run when the process runs. A reader who hits a stale answer needs a way
   to force the question from where they are standing — which, for a browser
   page, is the browser page. The escape hatch belongs in the interface the
   failure appears in, not only in the CLI the failure's owner happens to know.

## What it cost, and what it would have cost

One small cell each way: a `spawn_blocking` sweep over the registry after the
watcher spawns, and a form on the not-found page posting to a same-site-guarded
refresh route. Against that: an unknown number of readers over months quietly
concluding the viewer was broken, and one of them being the person who wrote it.

## Carry forward

- A startup reconcile is not defensive programming, it is the base case of the
  incremental algorithm. Ship it in the same cell as the watcher.
- Do not delay serving on it. Reconcile alongside the first requests and let the
  interface offer a manual reconcile for the window in between.
- When a stale-state failure has a fix that is one command long, the interface
  that showed the failure should be able to run that command. "The user can run
  X" is only true where the user can reach a shell.
