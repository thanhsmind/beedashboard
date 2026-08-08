---
artifact_contract: bee-plan/v1
mode: high-risk
# approved_gate2: <unset>
---

# Plan: Terminal Image Attach

Mode: `high-risk` — 1 risk flag: audit-security (an HTTP upload endpoint that
writes files to disk, on a server that binds 0.0.0.0)
Why this is the least workflow that protects the work: the only hard-gate
surface is the upload endpoint, so the plan concentrates its edge probes
there and keeps the UI slice on the existing composer patterns.

## Requirements (from CONTEXT.md)

- D1: Images come from the user's machine — file picker, drag-drop, and
  clipboard paste on the terminal pane page; multiple images per send.
- D2: The composer takes optional prompt text plus the attached image paths
  and submits them as ONE message with Enter.
- D3: Uploaded images land in a per-pane temp directory outside the repo;
  the sent message carries the file paths.

## Discovery

- `crates/mdview/src/server.rs:240` — `POST /p/:id/_terminal/:pane_id/input`
  already guards terminal switch + pane-in-project and sends text with
  submit; the attach endpoint copies these guards.
- `crates/mdview/assets/app.js:1150` (`sendReply`) — the composer form
  loop `.term-reply[data-pane-id]` (line 1159) posts JSON; attach wiring
  hangs off the same form.
- `Cargo.toml:41` — axum 0.7 without the `multipart` feature. Raw-body
  upload (one POST per file) needs no new dependency; multipart would.
  Verified: `grep -n 'axum' Cargo.toml`.

## Approach

Recommended (cites D1–D3): a raw-body upload route
`POST /p/:id/_terminal/:pane_id/attach` — one request per selected file,
body = file bytes, `Content-Type` = the image MIME type. The server
validates the declared MIME against an allowlist (png, jpeg, gif, webp)
AND sniffs the magic bytes for that type, enforces a 10 MB per-file cap
with an explicit length check answering in the same JSON error shape the
other terminal refusals use (a per-route `DefaultBodyLimit::max` layer
lifts axum's 2 MB default; the handler's own check owns the JSON answer),
and caps the pane's directory at 32 stored files. Files are written to
`<attach root>/<project>/<pane>/<random>.<ext>` where the attach root is a
user-owned, non-world-writable directory (`$XDG_RUNTIME_DIR/mdview-attach`
when set, else `~/.cache/mdview/attach`) — never bare `/tmp`, which is
1777 and symlink-preseedable — the project and pane segments are
sanitized to `[A-Za-z0-9-]` (pane ids carry `:`, illegal on NTFS), the
leaf name comes from `rand` (already a dependency; no `uuid` crate
exists in the graph), and the client's filename never reaches disk (D3).
Response: `{ "path": "<absolute path>" }`. The client keeps the returned
paths as removable chips on the composer; the Send action builds ONE
message — prompt text, newline, paths space-joined — and posts it to the
existing `input` route with `submit: true`, which is how D2's "with
Enter" lands in the pane; the composer's own keybindings (bare Enter =
newline, Ctrl+Enter = send) stay as they are. Picker, drag-drop onto the
composer, and paste in the textarea all feed the same upload function
(D1). The attach controls render only on project terminal pages — the
Unassigned page shares the `pane_cards` markup but has no project-scoped
attach route, so the markup is gated off there.

Rejected:
- axum `multipart` feature — new dependency surface for no capability gain
  at this file count.
- Base64-in-JSON upload — ~33% payload inflation and double buffering.
- Sending image bytes into the pane itself — terminals don't accept files;
  the agent reads paths.

Risk map: upload route / HIGH / edge-dimension probes below · composer JS /
LOW / covered by endpoint tests + manual page check · views markup / LOW /
existing render tests pattern.

## Shape

Epic map — outcome: from the terminal pane page a user attaches N images
and one prompt and the pane's agent receives one message naming N readable
paths. Basis: composer form, input route guards, and screen poller already
exist; only the upload leg and its UI are new.

| Epic | Capability/Risk Area | Why It Exists | Slices | Proof Needed |
|---|---|---|---|---|
| E1 | Upload endpoint (audit-security) | The one new trust-boundary surface | 1 | Guard + edge tests green |
| E2 | Composer attach UI | D1/D2 user surface | 1 | Render test + manual send on a live pane |

Slice queue: slice 1 = E1 + E2 together (walking skeleton — the surface is
user-visible, so the slice runs end-to-end: pick file → upload → chip →
send → message reaches pane). No later slices.

Current slice to prepare: 2 cells —
1. Attach endpoint in `server.rs`: guards, MIME allowlist + magic-byte
   sniff, 10 MB cap with JSON error contract, 32-file pane cap, sanitized
   user-owned attach root, `rand`-generated leaf names, plus its test set.
2. Composer UI: `views.rs` markup gated to project pages only +
   `assets/app.js` wiring (picker, drag-drop, paste, chips, send
   composition, and the client-visible rejection error surface), plus
   render test. Explicitly owns every user-facing error string.

## Test matrix

Edge dimensions applicable to the upload route (the hard-gate surface);
the rest of the 12 are N/A for a local dashboard feature with no roles,
compliance, or business rules beyond the allowlist:

- **User types / authorization**: terminal switch OFF → attach refused
  (same status as `terminal_input`); pane belonging to another project →
  refused; unknown pane → not found.
- **Input extremes**: zero-byte body → refused; body over the 10 MB cap →
  refused with the JSON error shape the composer can render; disallowed
  MIME (e.g. text/plain, image/svg+xml — scriptable) → refused; declared
  `image/png` whose bytes are NOT a PNG (magic-byte mismatch) → refused;
  allowed MIME with matching bytes → 200 with a path under the attach
  root carrying the matching extension.
- **Data integrity**: client filename with traversal characters never
  reaches disk (server generates the name; test asserts the written path's
  parent is the pane's attach dir); pane/project segments are sanitized
  (a `:` in the pane id never appears in the path); two uploads never
  collide (distinct random names); the 33rd file in one pane's dir is
  refused (count cap).
- **State transitions**: send with chips + empty prompt sends only paths;
  chips clear after a successful send; a removed chip's path is absent
  from the message (client-side, covered by the render test where
  assertable and the manual pass).
- **Error cascades**: herdr down at send time → same error surface the
  composer already shows; upload failure leaves no chip.

Happy path: upload two PNGs, send with a prompt, `pane.send_input`
receives one text containing prompt + both paths (asserted through the
fake herdr socket, the existing test pattern).

## Out of scope

- Picking images already inside the project tree (deferred in CONTEXT.md).
- Any agent-side handling of the paths.
- Cleanup daemon for the attach root (the 32-file pane cap bounds growth;
  `$XDG_RUNTIME_DIR` clears on logout where that root is used).
