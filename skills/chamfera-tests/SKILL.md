---
name: chamfera-tests
description: "Use when writing, placing or running tests in Chamfera, or when choosing the expected value for a geometry or kernel result."
---

# Tests

## Expected values come from outside the code
- Derive expected values by hand or analytically: a 50 x 30 x 10 box has volume 15000 mm^3; a cylinder r=5, h=10 has volume 250*pi. Write the derivation in a comment when it isn't obvious.
- Never paste "whatever the code printed" as the expected value.
- Never use values produced by commercial CAD (`skills/chamfera-ground-rules/SKILL.md`).
- Reference parts with analytic volume, area, centre of mass and bounding box live in `tests/oracle/` (from M3).

## Where tests go
- Unit tests next to the code in `#[cfg(test)] mod tests`, or in the crate's `tests.rs` (engine: `crates/engine/src/tests.rs`).
- A regression test goes into the area's existing test file, never a new per-bug file.
- Test names say the behaviour: `a_failing_command_changes_nothing_and_adds_no_undo_step`.

## Kernel and solver tests
- Check topology validity (Euler-Poincare: V - E + F = 2(S - H) for closed shells) as well as measures.
- Test near-degenerate cases: tangent faces, coincident edges, zero-length input, values at tolerance +/- epsilon.

## GPU tests
- Tests that render (in `crates/gpu`) skip on machines without any graphics adapter. CI sets `CHAMFERA_REQUIRE_GPU=1` and installs lavapipe, so there they must render; call `skip_without_gpu` in `crates/gpu/src/snapshot.rs` instead of skipping silently.
- Check pixels at positions derived from the camera (write the derivation), with a tight colour tolerance, away from edges and pixel-row boundaries.

## Running
- `cargo test --workspace` for everything, `cargo test -p chamfera-kernel` for one crate.
- `cargo xtask ci` before every commit.
