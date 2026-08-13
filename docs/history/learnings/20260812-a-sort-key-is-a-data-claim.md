# A locked decision that names a sort key is a claim about data that nobody checked

**Date:** 2026-08-12
**Found in:** cross-board — the Finished column's ordering, caught at plan review
**Applies to:** any shaping interview that offers the user an ordering,
grouping, filter, or threshold before the field behind it has been measured

## What happened

Shaping asked the user how the cross-project Finished column should be ordered
and offered "most recently shipped first" as the recommended option. They chose
it. It was locked as D6, cited in the plan, and would have gone into cells.

Two things were wrong with it, and neither was visible from inside the
interview:

1. **The per-project board it was supposed to match orders alphabetically.**
   `views.rs` sorts the feature list by slug and both the cards and the finished
   rows inherit that order. The screenshot the user supplied showed exactly
   that — Advisor, block-lean, Budget, capture-invariant, ci-cadence — and it
   was read as arbitrary rather than as the rule it was.
2. **There was no reliable ship timestamp to sort on.** The obvious field,
   `BeeShippedFeature.cycle_time.ended_at`, is `Option`, and it is computed from
   `.bee/cells/*.json` only — the archive, where most finished features live, is
   excluded by construction. Joining on it would have put roughly four in five
   finished features into an undated tail, which is precisely the outcome the
   decision existed to prevent.

The independent plan review found both. Measuring then found the real source:
`trace.capped_at` on the archived cells themselves, latest-wins per feature.
Across the eight qualifying projects that is 144 archived features and 346
archived cells, 140 of the 144 carrying a usable time, and a full scan costs
about 46 ms single-threaded on a warm cache. The decision was re-put to the user
with the true numbers and re-locked as D10 with an explicit two-block ordering:
timed features first, newest first, then everything else alphabetically.

## Why this generalises

An ordering question sounds like a preference question. It is not. "Newest
first" names a field, asserts that field is populated, and asserts it is
populated *often enough that the ordering means something*. A shaping interview
that asks it without checking is offering the user a choice whose cheaper option
may not exist — and once they pick, the answer is locked, cited downstream, and
expensive to reopen.

The same shape holds for grouping ("group by owner" — is owner set?),
filtering ("hide stale ones" — is there a timestamp to call stale?), and
thresholds ("warn over 100" — what is the actual distribution?).

## What to do instead

- **Before offering an ordering, grouping, or threshold option, check the field
  it rests on.** One targeted read of the struct and one count against real data
  is cheaper than a superseded decision. If the field is `Option`, find out how
  often it is `None` in practice, not in principle.
- **Check what the surface being copied actually does.** "Same as the existing
  one" is a decision about existing behaviour, so read the existing behaviour
  rather than inferring it from a screenshot. Sorted-looking output is often
  sorted by something other than what it looks like.
- **When the premise turns out false, go back to the user with numbers, not with
  an apology.** Re-asking with "140 of 144 have a usable time, the scan costs
  46 ms" got a better decision in one round than either guessing or quietly
  swapping the rule would have.
- **A superseded decision keeps its id.** D6 stayed in `CONTEXT.md` struck
  through with a pointer to D10 and a one-line reason, so a later reader sees
  that the ordering was decided twice and why.

## Evidence

- `crates/mdview/src/views.rs` — the per-project alphabetical sort the new board
  was supposed to match.
- `crates/mdview-core/src/bee.rs` — `BeeShippedFeature.cycle_time`, its `Option`
  doc comment, and the cells-only read that excludes the archive.
- `docs/history/cross-board/CONTEXT.md` — D6 superseded, D10 locked, and the
  measured numbers folded back into Outstanding Questions.
