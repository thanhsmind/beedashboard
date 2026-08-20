# One-tap Approve — Context

**Feature slug:** terminal-approve-button
**Date:** 2026-08-15
**Shaping session:** complete
**Scope:** Quick
**Domain types:** SEE

## Feature Boundary

Every terminal reply box grows an `Approve` button beside its `Stage`
button; one tap sends the word `Approve` to that pane and presses Enter,
exactly as typing it and hitting Send would. It ends there — no other
canned reply, no change to Stage, Send, or the key buttons.

## Locked Decisions

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | The button is labelled exactly `Approve`, is a `type="button"` with class `term-reply__approve`, and sits FIRST in the `.term-reply__actions` row: `Approve · Stage · Send`. | Leading the row keeps Stage and Send in their current relative positions, so existing muscle memory survives, and a one-tap send does not sit shoulder to shoulder with Send. |
| D2 | One tap sends for real, with no confirmation step: `POST {text: "Approve", submit: true}` to the same `/input` URL the Send button already posts to. | The user asked for one tap. Reusing the endpoint and its existing `submit` flag means no second send path is invented — the server still fires Enter as its own separate key event, as it does for Send today. |
| D3 | Approve ignores the textarea and any attach chips entirely: it sends only the word `Approve`, and whatever draft the user had typed is left untouched, never cleared. | Approve is a fixed answer, not a way to send the draft; clearing a draft the user did not ask to send would lose their work. |
| D4 | The button renders everywhere `Stage` renders: the project terminal page, the homepage Terminals tab, and the Unassigned terminal page. Because the Unassigned page is wired by its own inline `UNASSIGNED_TERMINAL_SCRIPT` rather than `assets/app.js`, the click handler must be added in BOTH places. | `pane_controls` is shared, so the markup appears on all three pages for free — but wiring only `app.js` would leave the Unassigned page's button inert. |

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Send-and-submit | `POST {text, submit: true}` — the server writes the text, then issues a separate `enter` key call. The repo calls this "send ≠ submit". |

## Existing Code Context

### Reusable Assets

- `crates/waggledance/src/views.rs:1225-1270` — `pane_controls`, the one
  renderer of the reply form; the `Stage`/`Send` pair lives at
  `views.rs:1258-1264`.
- `crates/waggledance/assets/app.js:1613-1624` — `sendComposed`, the
  existing `submit: true` post; `app.js:1672-1676` — the Stage click
  handler, the closest structural analog (a `type="button"` sibling that
  posts to `/input`).
- `crates/waggledance/assets/app.js:1474-1481` — `postJson`, and
  `inputUrl(paneId, base)`, which already resolves the right URL for both
  the project page and the homepage tab's `data-term-base`.

### Established Patterns

- The key-button row (`views.rs:1251-1256`, handler `app.js:1705-1715`) is
  the precedent for a button that sends a hardcoded constant with no
  textarea involved — though it posts to `/keys`, not `/input`.

### Integration Points

- `crates/waggledance/src/views.rs:1544-1671` —
  `UNASSIGNED_TERMINAL_SCRIPT`, the deliberate duplicate of `app.js`'s
  wiring for the Unassigned page (documented at `views.rs:1535-1543`),
  which posts to `/_terminal/unassigned/:pane_id/input`.
- `crates/waggledance/src/views.rs:704` — the shared
  `.term-reply__send, .term-reply__stage` CSS rule the new button joins,
  and the mobile tweak at `views.rs:678`.
- `crates/waggledance/src/server.rs:2280-2298` — `terminal_input`, unchanged
  by this feature; its `submit: true` behaviour is already covered by
  `terminal_input_with_submit_sends_text_then_enter` (`server.rs:12323`).

## Outstanding Questions

None.

## Deferred Ideas

- More canned replies (Reject, Retry) beside Approve — one button is what
  was asked for; a row of them is a different feature. Revisits on
  `a-user-asks-for-a-canned-reply-beside-ap__3be024da`.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
