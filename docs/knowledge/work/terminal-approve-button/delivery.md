---
type: bee.delivery
title: terminal-approve-button — delivery
description: "Delivery record for work item terminal-approve-button: every terminal reply box gained a one-tap Approve."
timestamp: 2026-08-15
bee:
  id: terminal-approve-button-delivery
  lifecycle: active
  areas: [agent-terminal, web-interface]
  required_context: [docs/specs/agent-terminal.md, docs/specs/web-interface.md]
  sources: [docs/history/terminal-approve-button/CONTEXT.md, .bee/cells/archive/terminal-approve-button/terminal-approve-button-1.json]
---

# terminal-approve-button — Delivery

## What shipped

Approving an agent's proposal meant typing the same word every time and
pressing send — on a phone, the most common reply in the system was also one
of the slowest.

Every terminal reply box now leads its button row with `Approve`, ahead of
Stage and Send. One tap sends the word `Approve` to that terminal and
presses Enter, exactly as typing it and hitting Send would.

- **It sends for real.** There is no confirmation step. That was the
  explicit choice: one tap or it is not worth having.
- **It ignores what you were writing.** The button never reads the reply box
  and never attaches whatever files were staged; it sends only the one word.
  A half-written draft survives the tap untouched, because clearing work the
  user never asked to send would be losing it.
- **Everywhere the Stage button appears.** The project's terminal page, the
  home page's Terminals tab, and the page for terminals belonging to no
  project. One renderer draws the button on all three; the click behaviour
  had to be wired twice, because the unassigned page carries its own copy of
  the page script rather than sharing the main one.

Nothing changed on the sending side. The button rides the same path Send
already used, which writes the text and then issues Enter as its own
separate key press — the system's standing distinction between placing text
in a terminal and submitting it.

## Verify

`cargo test --workspace` green at 980, up from 976. Cases cover the button
rendering ahead of Stage — asserted on relative position, not mere presence —
the shared button sizing rule naming it alongside its two siblings so it
cannot drift to a different size or touch target, and both wiring copies
sending the exact word with the submit flag set. The submit path itself was
already covered by the existing test that a submitted reply sends text and
then Enter.

## Deviations

None recorded.

## Provenance

Written from the capped trace of `terminal-approve-button-1` and the locked
decisions in `docs/history/terminal-approve-button/CONTEXT.md`. Sits in the
terminal reply box, beside the fixed-key buttons
[ctrl-c-key](../ctrl-c-key/index.md) added.
