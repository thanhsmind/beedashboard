# A guard test must fail when its own guard is removed

**Date:** 2026-08-06
**Found in:** agent-terminal — six independent review passes across six slices
**Applies to:** any test standing in for a security control
**Extends:** [20260805-toothless-security-assertions.md](20260805-toothless-security-assertions.md)

## The number that matters

Twenty-five cells, 480 passing tests, six independent review passes. The reviews
found two live authentication holes, two fixes that did not fix what they
claimed, four tests that named one guard while passing on another, a fail-open
listing, a path escape, and several correctness defects.

**The suite caught none of them.** It was green while every one of them was live.

That is not an argument against tests. It is an argument that a green suite is
evidence of nothing in particular until you know which mutation each test would
catch.

## The recurring shape

A handler carries several guards in sequence — method, session, feature switch,
containment. A test named for one of them supplies a request that *any* of them
would reject. It passes. Delete the guard it is named for and it still passes,
because a different guard caught the request first.

Concretely, from this feature:

- `terminal_route_without_a_session_...` ran with the feature switch **off**, so
  the switch answered before the session check ever ran. Removing the session
  extractor left it green.
- The wrong-method tests sent no cookie, so the session check answered before the
  method gate. Removing the method gate left them green.
- The three `Unassigned` routes had no guard test at all — every request in the
  suite carried a valid cookie. That was the one route family with no containment
  check to fall back on.

## The rule

**A guard test must isolate its guard: every other guard in the chain must be
satisfied, so the request can only be refused by the one under test.** A
no-session test runs with the feature switched *on*. A wrong-method test runs
with a valid session *and* the feature on.

And the only way to know is to check:

> Before trusting a guard test, delete the guard it names, run the test, confirm
> it goes red, and restore. If it stays green, it is decoration.

Every fix cell in the second half of this feature carried that instruction, and
the workers who followed it reported which guards they had verified that way —
which is how the last three slices stopped producing this defect.

## Two fixes that did not fix anything

Worth naming separately, because both looked finished and both had tests:

- An in-flight guard added to stop a poller double-appending cleared its flag when
  response *headers* arrived, while the cursor advanced only after the *body*
  parsed. The race it existed to close ran straight through the gap.
- A truncation check added to stop silent record loss left the cursor at a
  non-boundary offset in one case, which the next poll read as a fresh truncation
  — turning a silent loss into a visible storm several times a second.

Both were written against the description of the bug rather than against the
mechanism, and both had a passing test. **"Fixed" and "proven" are different
claims.** A fix earns the second only by naming the mutation that would break it.

## Cheapest checks, in order

1. For each guard in a chain, one test that isolates it — others satisfied.
2. Delete-the-guard, confirm red, restore. Record which ones you did this for.
3. For a source-text assertion (no JS runner here), make the matched string
   unique to the behaviour. `js.contains("inFlight[paneId] = false;")` matched two
   different sites, so deleting one still passed.
4. `assert_ne!` on two source *lines* that differ only by variable name can never
   fail. If an assertion has no failing input, it is not a test.

## Where this bit hardest

The routes with no fallback. Project-scoped routes have a containment check that
would refuse a wrong pane even if the session gate vanished. The `Unassigned`
family has no such check by design — the panes there belong to no project — so its
session gate is load-bearing alone, and it was the family with zero guard tests.

**Rank guard tests by what fails if the guard is gone, not by how many the route
has.**
