PROCEED WITH CONDITIONS — high-risk advisor consult, bee-board-pm, 2026-08-06.

Lane: high-risk confirmed, carried by proof-weakening and covered-contract-change (28 markup tests deleted beside security-shaped D4/D9 invariants). public-contracts is the weakest flag (one consumer, additive struct changes, HTTP routes unchanged); multi-domain is light. Drop both and the lane is still high-risk.

Conditions, all required before code is written:

1. Every new reader ships its own read-only fixture with its file PRESENT and populated. The six existing read-only tests each fixture only their own section's files; none contains `HANDOFF.json`, `config.json`, `capture-queue.jsonl`, `review-candidates.jsonl`, `reviews/*.json` or `reservations.json`, so every new reader takes its missing-file early return and the tests pass without ever running the code that could write.

2. `snapshot_tree` (`server.rs:2414-2432`) records directories as well as files, in S1a, with all six existing read-only tests green over the change. It pushes an entry per file and none per directory, so a `create_dir_all` before a `read_dir` — the obvious idiom for listing `.bee/reviews/*.json` — writes into the user's store while the assertion passes.

3. `gate_bypass` resolution settled before coding. `.bee/config.local.json` is a machine-local overlay ("the effective (overlay-over-tracked) value"); the board renders what `config.json` literally records and labels it as such. A wrong effective-level claim is worse than no claim, inside the panel whose job is trust.

4. Feature-name to path joins validated at the join site — no separators, no `..`, not absolute — with a traversal-shaped `feature` probe in a cell and in `state.json`. `docs/history/<feature>/promote-proposals.md` is this area's first store-string-to-path join; `feature` is unvalidated free text (`bee.rs:680-684`) and the nearest precedent (`resolve_worktree`, `bee.rs:1073`) joins unvalidated too, so house style would copy the hole. Presence-rendering alone makes the board a filesystem oracle for paths outside a project mdview does not own.

Also found: Discovery's `state.json` enumeration missed `gate_revoked_at`, which feeds the lifecycle stepper directly.

Recommended, not a condition: split S1 into S1a (scrubber, `snapshot_tree`, `state.json` fields already in an open file, full D5 skeleton, the rules existing data supports) and S1b (`HANDOFF.json` + `config.json` readers, their builders, their read-only fixtures, the stale-handoff and bypass rules). S1b is the only part introducing new file-opening code.

Judged sound: the reader-side derivation seam, the D9 correction already absorbed from the prior plan-check pass, and the rules-that-must-survive table (the strongest part of the plan).

Disposition: all four conditions and the S1a/S1b split were written into `docs/history/bee-board-pm/plan.md` before the gate was recorded.

Consult identity: bee-review subagent, review tier, read-only, dispatched 2026-08-06 with the evidence bundle CONTEXT.md + plan.md + docs/specs/bee-cockpit.md + the three code files. Recorded because this repo configures no `models.claude.advisor`; the gap is named here rather than skipped.
