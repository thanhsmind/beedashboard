---
artifact_contract: bee-implement-plan/v1
feature: terminal-image-attach
lane: high-risk
status: Ready for Review
updated: 2026-08-08
sources: [CONTEXT.md, plan.md]
decisions: [D1, D2, D3]
---

# Implementation Plan: Terminal Image Attach

> Human-layer projection of the truth artifacts. Truth lives in CONTEXT.md
> (decisions), plan.md + cells (work), and test-result records (evidence).
> Feedback on this document flows back to those artifacts, then this re-renders.

## 1. Goal

From the terminal pane page, the user attaches several images from their own
machine, types an optional prompt, and presses Send once — the agent running
in that pane receives one message naming every image path, ready to read.

**Success looks like**
- Images arrive via file picker, drag-drop, or clipboard paste, several at a
  time (D1).
- One Send delivers prompt text plus all attached paths as a single
  submitted message (D2).
- The image files live in a per-pane directory outside the repo; only their
  paths travel to the pane (D3).

## 2. Current State

The terminal pane page already has a working composer: a `.term-reply` form
per pane posts JSON to `POST /p/:id/_terminal/:pane_id/input`
(`crates/mdview/src/server.rs:240`), which guards the terminal switch and
pane-project membership, then calls herdr's `pane.send_input`
(`crates/mdview/src/herdr/socket.rs:523`). There is no upload surface
anywhere: axum 0.7 runs without the multipart feature and the router has no
file-accepting route. The Unassigned terminal page shares the composer
markup (`pane_cards`, `crates/mdview/src/views.rs:680`) but has its own
input route and no project scope.

## 3. Scope

**In scope**
- One new upload route, project- and pane-scoped, with security guards (D3).
- Composer additions on project terminal pages: attach button, drag-drop,
  paste, removable chips, one-message send composition (D1, D2).

**Out of scope**
- Picking images already inside the project tree (deferred in CONTEXT.md).
- Any agent-side handling of the paths.
- A cleanup daemon for stored files (the 32-file pane cap bounds growth).

## 4. Proposed Approach

One raw-body POST per selected file to
`/p/:id/_terminal/:pane_id/attach`; the server validates and stores the
file, returns its absolute path, and the client collects paths as chips.
Send posts one message — prompt, newline, paths — to the existing `input`
route with `submit: true`, which is how "with Enter" (D2) reaches the pane;
composer keybindings are untouched.

**Why this approach** — reuses the existing input route and composer form;
adds exactly one route and zero dependencies (`rand` already provides the
name randomness; axum's `Bytes` extractor and `DefaultBodyLimit` are not
feature-gated).
**Alternatives considered** — axum multipart (new dependency surface, no
capability gain); base64-in-JSON (payload inflation, double buffering);
sending bytes into the pane (terminals take text, not files).

## 5. Technical Design

```text
picker/drop/paste -> app.js upload (1 POST per file, raw bytes)
  -> attach route: switch guard -> pane-in-project guard -> MIME allowlist
     -> magic-byte sniff -> 10 MB cap -> 32-file pane cap
  -> write <attach-root>/<project>/<pane>/<random>.<ext> -> { "path": ... }
  -> chip on composer
Send -> existing input route (text + paths, submit: true) -> pane.send_input
```

- **API / contract** — `POST /p/:id/_terminal/:pane_id/attach`, body = file
  bytes, `Content-Type` = declared image MIME. Success: `200 { "path" }`.
  Every refusal uses the JSON error shape the terminal routes already use
  (the body-limit case is answered by the handler's own length check so the
  composer can render it, not by axum's plain-text 413).
- **UI / UX** — attach button + chip list on the `.term-reply` form,
  project terminal pages only (the Unassigned page shares the markup but
  has no project-scoped route, so the controls are gated off there).
  Upload failure shows the error beside the composer and leaves no chip.
- **Security / Permissions** (mandatory, high-risk) —
  - Attach root is user-owned and never world-writable:
    `$XDG_RUNTIME_DIR/mdview-attach` when set, else
    `~/.cache/mdview/attach` — never bare `/tmp` (1777, symlink-preseedable).
  - Path segments: project and pane ids sanitized to `[A-Za-z0-9-]`
    (pane ids carry `:`); leaf name generated from `rand`; the client's
    filename never reaches disk.
  - Content: declared MIME allowlist (png, jpeg, gif, webp) AND magic-byte
    sniff — `image/png` declaring non-PNG bytes is refused; svg is excluded
    as scriptable.
  - Abuse limits: 10 MB per file, 32 stored files per pane; both answer in
    the JSON error shape.
  - Same switch + membership guards as `terminal_input` — the route grants
    no access the input route does not already grant.

## 6. Affected Files

| Action | File / Component | Purpose |
|--------|------------------|---------|
| Modify | `crates/mdview/src/server.rs` | attach route, guards, storage, tests |
| Modify | `crates/mdview/src/views.rs` | composer attach markup, project pages only |
| Modify | `crates/mdview/assets/app.js` | upload wiring, chips, send composition, error surface |

## 7. Implementation Steps

- [ ] Attach endpoint with guards, allowlist + sniff, caps, sanitized
  user-owned root, `rand` names, JSON error contract, and its test set.
- [ ] Composer UI: gated markup + JS wiring (picker, drag-drop, paste,
  chips, send composition, visible errors) and render test.

## 8. Validation Plan

**Automated** — `cargo test --workspace` → expected: existing suite stays
green; new tests cover the edge-dimension matrix in plan.md (guards, size
and count caps, MIME/sniff refusals, sanitized paths, no-collision names,
happy path asserting one `pane.send_input` text carrying prompt + paths
through the fake herdr socket).
**Manual** — [ ] on a live pane: pick two images, drop one, paste one,
remove a chip, send with a prompt, confirm the agent sees the paths;
confirm the Unassigned page renders no attach controls.
**Evidence** — pending.

## 9. Risks & Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| Upload route abused (traversal, spoofed content, disk fill) | High | Sanitized server-generated paths, magic-byte sniff, size + count caps, user-owned root — each pinned by a test |
| Composer regressions on the shared markup | Med | Attach markup gated to project pages; render tests on both pages |
| Cap errors invisible in UI | Low | Handler owns the JSON error shape; client renders it beside the composer |

## 10. Rollback Plan

Revert the slice's commits (one per cell) in the feature worktree or drop
the worktree before merge — no migration, no config, no external system.
Already-stored files under the attach root are inert and removable by
deleting the directory.

## 11. Open Questions

No blocking open questions. Ready for review.
