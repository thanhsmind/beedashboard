# Learning: A lower-bound assertion proves nothing about a mechanism

**Category:** failure
**Severity:** standard
**Tags:** [tests, verify, review, mutation]
**Applicable-when:** a test asserts `>=`, `any()`, or presence where the policy under test implies an exact count, an order, or a spacing.

## What Happened

The independent review of terminal-attach-submit-race ran a mutation battery
over the settle-wait tests and found all three of the wait's mechanisms
individually deletable with a green suite: the stop-on-repeat return (test
asserted only `reads >= 3`), the min-quiet sleep (no test observed when the
first read arrived), and the poll-interval sleep (the never-settles test
only checked that *some* read happened). Three P1s from one defect class.
Each was fixed by turning the loose claim into the exact one the policy
implies: an exact read count with a panic-on-extra mock, a timestamp gap
assertion on the first read, a min-gap plus read-count ceiling on the poll.
Every fix was proven by re-running the mutation: delete the mechanism,
quote the red.

## Root Cause

Lower-bound and presence assertions describe what the code *at least* does,
so they stay green when a mechanism is removed and the behavior degrades to
something that still clears the floor. The suite reads as coverage while
guarding nothing: the wait could become a fixed sleep, a busy-spin storm, or
always-run-to-cap without a single red. Green-path testing cannot see this —
only deleting the mechanism can.

## Recommendation

When a test guards a mechanism (a wait, a retry, a dedup, a throttle),
assert the exact shape the policy implies — counts with `assert_eq`, order
with first/last/between, spacing with timestamps — and verify the assertion
by mutation: delete the mechanism in a scratch copy and demand a red. A
mock that panics on the extra call is cheaper than a clever assertion. Do
this at review time for any timing- or scheduling-shaped diff; it found
three P1s here that two green suites and a live measured run all missed.
