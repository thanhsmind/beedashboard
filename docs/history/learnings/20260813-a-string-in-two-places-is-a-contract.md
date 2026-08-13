# A string written in two places is a contract

2026-08-13 · feature `waggledance-rename` · 9 capped cells, 886 → 906 tests

Renaming `mdview` to `waggledance` was supposed to be mechanical. The compiler catches
identifiers; a `sed` catches prose. What it does not catch — and what nearly shipped
three times in one feature — is a **string literal written independently in two places
that must agree**.

## The three

1. **The daemon health handshake.** `server.rs` answered `/health` with `"app": "mdview"`.
   `daemon.rs` detected a live daemon by testing whether the response contained that quoted
   string. Two crates, two literals, no shared constant. Rename one side and there is no
   error, no log — auto-spawn simply stops recognising a daemon that is already running and
   starts fighting it.
2. **The MCP registration key.** `doctor` used the literal `"mdview"` as *both* the lookup
   key and the inserted key. Renaming the constant means the "already registered" check no
   longer sees the old entry, so `doctor --fix` adds a second server and leaves the first
   one pointing at a binary that no longer exists.
3. **The managed-block markers.** `<!-- mdview:START -->` was found by literal
   `text.find`. Rename it and the writer takes its "not found" branch and **appends** a
   second block below the stale one.

Same shape every time: a value that is really one contract, stored as two copies, where
disagreement is silent rather than loud.

## What actually fixed it

Not renaming both sides carefully — that only postpones the next occurrence. The fix is a
test that **compares the two sides to each other**:

- The health test hits the real `/health` route and asserts the body satisfies
  `daemon.rs`'s own `looks_like_daemon` predicate — extracted for exactly this purpose.
  Neither side of that test hardcodes the name.
- The mermaid-done event test compares the dispatch site to the listener, anchored on an
  unrelated callback name so two agreeing hardcoded copies cannot pass it.

A test that hardcodes the new name twice proves nothing; it will agree with itself through
the next rename too.

## Second lesson: a migration guarded on "target absent" is disarmed by any mkdir upstream

The plan put the one-time `~/.mdview` → `~/.waggledance` move at daemon startup, guarded
on "new directory absent and old directory present". Review found that unreachable:
`create_dir_all` runs unconditionally on the way to the registry, the config, and the
daemon lock, and half a dozen entry points reach one of those before any daemon starts —
including `serve` itself, which persists its `--port` before serving. The first command a
user ran would have created the new directory, disarmed the guard permanently, and
orphaned the old registry in silence.

**A one-time migration belongs at the resolver, not at an entry point** — there is no
"first" entry point, and the thing that creates the directory always runs before the thing
that would have migrated it.

Related: the binary installed *inside* the directory being migrated
(`$HOME/.mdview/bin`, and `%USERPROFILE%\.mdview\bin` as the Windows **default**). Renaming
a directory that holds the running executable is `ERROR_SHARING_VIOLATION` on Windows.
Moving the install location out was a precondition for the migration existing at all.

## Third: for a test-suite escape hatch, prefer opt-in to opt-out

The migration needed to never fire during `cargo test`. The cell asked for an opt-out the
suite sets. The worker built an opt-in instead, armed only inside `cli::run` — the one
dispatch point every real subcommand passes through, and nothing a test calls.

An opt-out is one missed call site away from renaming a developer's real home directory,
and dozens of route-level tests resolve the data directory. When the failure mode is
destructive and the call sites are many, default to inert and arm deliberately.

## Also worth remembering

- `bee cells finish` runs the declared suite **in the main checkout**, which does not
  carry a feature worktree's work. Its "tests: green" corroborates; the worker's run inside
  the worktree is the evidence. Record which is which.
- `find` and `grep` in this session pass through a filtering proxy that hid a leftover
  empty `crates/mdview-core/` directory. `rtk proxy <cmd>` shows the true output. A
  completeness sweep is only as good as the tool running it.
- A completeness sweep must cover **path names and lockfiles**, not just file contents. Two
  renamed directories and a report filename carried the old name in paths that no content
  grep could ever find.
