# Terminal Image Attach — Context

**Feature slug:** terminal-image-attach
**Date:** 2026-08-08
**Shaping session:** complete
**Scope:** Quick
**Domain types:** SEE | CALL

## Feature Boundary

On the terminal pane page, the user can attach multiple images from their own
machine (file picker, drag-drop, clipboard paste), type an optional prompt, and
send everything to the pane as one message; the feature ends at the submitted
message — what the agent in the pane does with the paths is out of scope.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | Images come from the user's machine: file picker, drag-drop, and clipboard paste on the terminal pane page; multiple images per send | Upload-from-device chosen; project-tree picking offered and declined |
| D2 | The composer takes optional prompt text plus the attached image paths and submits them as ONE message with Enter | Prompt-box variant chosen over paste-only and auto-submit |
| D3 | Uploaded images land in a per-pane temp directory outside the repo; the sent message carries the file paths | Repo stays clean; the agent in the pane can still read the files |

### Agent's Discretion

- Accepted image types, per-file size cap, and per-send count cap — pick
  sensible defaults and surface a visible error on rejection.
- Exact temp path layout and any cleanup policy.
- Composer UI placement and look, following the existing terminal page style.

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/mdview/src/server.rs:240` — `POST /p/:id/_terminal/:pane_id/input`
  (`terminal_input`) already sends text to a pane with submit.
- `crates/mdview/src/herdr/socket.rs:523` — `send_input(pane_id, text, submit)`
  wraps herdr's `pane.send_input`.

### Established Patterns

- Terminal endpoints gate on the terminal switch and validate the pane belongs
  to the project (see `terminal_screen` / `terminal_input` guards and their tests).
- Client JS in `crates/mdview/assets/app.js` uses `fetch` against the terminal
  endpoints; the pane page renders per-pane controls.

### Integration Points

- `crates/mdview/src/server.rs` — new upload route beside the terminal routes.
- `crates/mdview/assets/app.js` + the terminal pane page template — composer UI.

## Outstanding Questions

### Deferred To Planning

- [ ] Multipart upload support in the current axum setup — check the feature set
  of the axum version in `Cargo.toml`.
- [ ] How the paths are formatted in the sent message (spacing/quoting) so the
  agent CLI in the pane reads them cleanly.

## Deferred Ideas

- Picking images already inside the project tree — declined for now.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
