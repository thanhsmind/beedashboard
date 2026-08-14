---
type: bee.delivery
title: term-url-links — delivery
description: "Delivery record for work item term-url-links: 3 capped cell(s), 2 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: term-url-links-delivery
  lifecycle: active
  areas: [agent-terminal, web-interface]
  required_context: [docs/specs/agent-terminal.md, docs/specs/web-interface.md]
  sources: [.bee/cells/archive/term-url-links/term-url-links-1.json, .bee/cells/archive/term-url-links/term-url-links-2.json, .bee/cells/archive/term-url-links/term-url-links-3.json]
---

# term-url-links — Delivery

## What shipped

An agent prints addresses constantly — the page it just changed, the health
check it just ran — and on a phone the only way to follow one was to read it
off the screen and type it back in. Document paths had been clickable for a
while; addresses had not.

A web address on a terminal screen is now a link that opens in a new tab, the
same way a document path already did. The rule is deliberately narrow: the
address must say `http://` or `https://` itself. A bare hostname, or a host
and port with no scheme, stays plain text — an agent's ordinary output is full
of things that look like hostnames and are not, and a link that guesses wrong
is worse than no link.

Three things the match has to get right, each learned from the running
screen rather than from the plan:

- Sentence punctuation stays outside the link, so an address ending a sentence
  does not carry the full stop into the address.
- A scheme with nothing behind it is prose, not a link. Text explaining the
  rule — naming the two schemes — was itself being turned into a link to
  nowhere.
- The screen text arrives already escaped for the browser, and an address
  written inside quotes therefore ends with an escaped quote whose every
  character is legal inside an address. The match stops at any such escape,
  except the one that stands for a real ampersand, which belongs inside a
  query string.

An address the terminal wrapped across two rows is not rejoined; each row is
matched on its own.

Terminals belonging to no registered project get the same treatment. They had
no link handling at all before.

## Verify

`cargo test --workspace` green at 954, up from 947, plus three regression
tests added by the two follow-up fixes.

Confirmed against the running daemon three times — once after each cell.
The first run exposed the bare-scheme link, the second the swallowed escape,
and the third shows addresses linking cleanly with their surrounding
punctuation and quoting left outside.

## Deviations

The two follow-up fixes ran inline rather than through a dispatched worker,
recorded on each cell as the small-fix allowance: both were one function and
its tests, opened by a regression visible on the live screen.

## Provenance

Written from the capped traces of `term-url-links-1`, `-2` and `-3`. The
narrow matching rule — schemes only, no bare hostnames — was the user's own
choice and is recorded in the decision log.
