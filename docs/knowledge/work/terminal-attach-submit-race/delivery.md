---
type: bee.delivery
title: terminal-attach-submit-race — delivery
description: "Delivery record for work item terminal-attach-submit-race: a Send carrying an attached image now waits for the terminal to settle before pressing Enter, so it sends instead of sitting in the composer."
timestamp: 2026-08-16
bee:
  id: terminal-attach-submit-race-delivery
  lifecycle: active
  areas: [agent-terminal, web-interface]
  required_context: [docs/specs/agent-terminal.md]
  sources: [.bee/cells/terminal-attach-submit-race-1.json, .bee/cells/terminal-attach-submit-race-2.json]
---

# terminal-attach-submit-race — Delivery

## What was wrong

Attaching an image in the web reply box and pressing **Send** looked like
pressing **Stage**: the message — image and text together — appeared in the
agent's composer and stayed there, unsent, until somebody pressed Enter in
the terminal itself.

Send was doing its part correctly. It composed the reply and asked for a
real submit; the request even carried the flag that means "press Enter".
The loss happened one layer down, in how that submit reaches the terminal.

Placing text and submitting it are two separate acts here — the text is
written first, then Enter is sent as its own keystroke. That separation is
deliberate and load-bearing, but the two were fired back to back with
nothing in between. When the reply carries an image, the agent has to read
that image off disk and turn the path into an attachment chip before it can
accept anything else; the Enter arrived in the middle of that work and was
swallowed. A reply with no image had nothing to wait for, which is why plain
Send always worked and only attachments broke.

## What now happens

Between writing the text and pressing Enter, the sender waits for the
terminal to stop changing:

- a **quiet window of 250ms** first, long enough that a slow image read has
  begun and disturbed the screen — waiting for stillness before the screen
  has even flinched would settle on the wrong moment;
- then it **watches the screen** every 100ms and treats it as settled the
  first time two consecutive looks are identical;
- with a **hard cap of 1.5 seconds** measured from the text write. On the
  cap, on a read failure, on any error at all, the Enter is sent anyway.
  A person's submit is never dropped because the screen would not hold
  still — the worst case is the old behaviour, never a silently lost reply.

Measured against a real agent terminal: the wait ended at 352ms with the
image already resolved into a chip in the composer, and the Enter submitted.

## What "the screen stopped changing" means

It means the screen's **text** repeated, not that a revision counter held
still. The first version of this fix compared the revision number the
terminal reports with each read, which was wrong in a way that hid itself:
that number is always zero from the real terminal server. Eight consecutive
reads taken while an agent was actively printing output all reported
revision zero while the text underneath changed on nearly every one, and
every terminal in the workspace listing reports zero as well.

Comparing revisions therefore always "settled" on the second look, and the
wait quietly degenerated into a fixed floor of about 350ms. That floor
happened to be long enough for the image that exposed the bug, which is
exactly what makes it dangerous: a larger image, a slower disk, or a busier
machine would have raced again with nothing in the code to show why. The
comparison is on the text now, and the reason is written where the next
reader will find it before deciding to simplify it back.

## What did not change

- Text and Enter still reach the terminal as two separate requests. Fusing
  them would leave every reply unsent in the composer.
- Staging is untouched: a Stage still writes the text and presses nothing,
  with no waiting at all.
- A submit with no text — the Approve button's shape — still sends only the
  Enter, immediately.
