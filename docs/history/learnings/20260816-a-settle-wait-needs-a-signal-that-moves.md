# Learning: A settle wait needs a signal that actually moves

**Category:** failure
**Severity:** standard
**Tags:** [polling, verify, terminal]
**Applicable-when:** code waits for "no change" by comparing a reported counter, revision, or timestamp between polls.

## What Happened

`terminal-attach-submit-race-1` made the web Send wait for the pane's screen
to settle before pressing Enter, defining "settled" as two consecutive
`pane.read` calls reporting the same `revision`. Measured against the real
herdr daemon, that field is dead: eight consecutive reads taken while an
agent was actively streaming output all answered revision 0 while the screen
text changed under them, and every pane in the workspace listing reports 0
too. The loop therefore always "settled" on its second look, and the wait
quietly degenerated into a fixed ~350ms floor — long enough for the image
that exposed the bug, which is exactly what made it dangerous: a larger
image, slower disk, or busier machine would have raced again with nothing in
the code to show why. `terminal-attach-submit-race-2` re-based the comparison
on the screen's text itself.

## Root Cause

The settle signal was chosen from the API's shape, not from observed
behaviour. A field named `revision` promises change-tracking; nothing checked
that the server actually increments it. A wait keyed to a constant signal
does not fail — it silently becomes a sleep, and the test suite cannot tell a
working settle-detector from a fixed delay that happens to be long enough
today.

## Recommendation

Before keying any quiet-window / settle / debounce logic to a reported
counter, probe the live system and watch the field move while the underlying
thing is actually changing. If it does not move, compare the content itself
(hash it when it is large). When a degenerate signal is discovered, write the
reason into the code where the comparison happens, so the next reader does
not "simplify" it back to the cheap dead field.
