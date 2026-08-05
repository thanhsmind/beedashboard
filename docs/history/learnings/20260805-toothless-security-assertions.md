# A security assertion that names a literal path has no teeth

**Date:** 2026-08-05
**Found in:** bee-cockpit planning review, before any code was written
**Applies to:** any test that asserts sensitive data does not appear in output

## What happened

The plan for the bee cockpit carried a HIGH-risk row — mdview's server has no
authentication, and bee's store is full of absolute filesystem paths. The test written
to guard it asserted:

```rust
assert!(!html.contains("/home/"));
```

The review wave killed it. Fixtures in this workspace build under
`std::env::temp_dir()` (`crates/mdview/src/runtime.rs:249`). A fixture project at
`/tmp/bee-fix-123` whose cells carry `files[] = ["/tmp/bee-fix-123/src/a.rs"]` renders
those paths verbatim into the page — and the assertion passes green. The one test
guarding the highest risk in the plan could not fail for the shape it existed to catch.

## The rule

A leak assertion must be written against **the value that would leak**, not against a
literal that happens to appear in production. Two forms, both used in the shipped tests:

```rust
// the fixture's own root — the thing that must not escape
assert!(!body.contains(&root.to_string_lossy().into_owned()));
// and the general shape
assert!(!Path::new(field).is_absolute());
```

`crates/mdview-core/src/bee.rs` (`no_absolute_path_survives_into_public_fields`) and
`crates/mdview/src/server.rs` (the route-level probe) both carry this form.

## The wider pattern

The same defect shape recurs wherever a test asserts an absence:

- Asserting on a hardcoded secret string instead of the fixture's generated secret.
- Asserting a redaction removed `"password"` when the field is named `pwd` in the
  fixture.
- Asserting an error message does not contain `"/etc/"` when the fixture writes to a
  temp dir.

**Check:** before trusting an absence assertion, change the production value in the
fixture and confirm the test goes red. If it stays green, the assertion is decorative.

## Second finding from the same review

`crates/mdview/Cargo.toml` had **no `[dev-dependencies]` section at all**, so there was
no way to drive `router()` — every existing test in `server.rs` was a pure-function
test. A plan that says "covered by the existing test module pattern" must check that the
pattern can reach the thing being tested. Here it could not, and the route-level half of
a locked decision (no `.bee/` → not-found, not an empty page) would have gone unproven.
Adding `tower` + `http-body-util` as dev-deps was folded into the cell.
