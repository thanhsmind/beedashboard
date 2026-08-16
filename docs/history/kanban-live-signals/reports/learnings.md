# kanban-live-signals — Learnings (2026-08-16)

- **Subagent workers inherit the orchestrator's session root.** A bee-build
  worker dispatched from a main-rooted session cannot write into the feature
  worktree — the write guard refuses every edit and no CLI verb re-roots a
  running session. The working fix: the orchestrator enters the worktree
  itself (harness EnterWorktree) before dispatching, so workers inherit the
  right root. First dispatch of cell 1 was lost to this.
- **Register the worker before dispatch.** `bee cells finish` checks the
  capping worker against `state.workers[]`; a worker the orchestrator never
  added must repair it mid-cell with `bee state worker add`. Registering at
  claim time avoids the detour.
- **Control-plane verbs refuse inside a granted worktree** (cells claim/finish,
  close) and name main as the place to run them — worker prompts must carry
  the `cd <main> && bee …` shape or the worker burns turns rediscovering it.
- **`deferred-queue.jsonl` carried only `add` events** at implementation time.
  The reader treats "last event is add" as unresolved and any later event for
  the id as resolving; if bee later grows explicit resolve/flush event kinds
  with different semantics, revisit the fold in
  `crates/waggledance-core/src/bee.rs`.
