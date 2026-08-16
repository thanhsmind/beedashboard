# home-terminal-header — Learnings (2026-08-16)

- **The repo has no JS test harness** — cell 2 (drawer unification in
  assets/app.js) could only be regression-checked by the Rust suite and
  verified by a browser pass against the running daemon. Named gap: JS-side
  behavior ships on manual verification until a harness exists.
- Rust-side UI contracts (which surface offers which creation control) are
  cheap to pin in server/view tests; the homepage-vs-project-page split caught
  its edge case (zero presets ⇒ no creation box at all) that way.
