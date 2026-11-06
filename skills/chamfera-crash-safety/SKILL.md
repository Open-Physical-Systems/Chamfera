---
name: chamfera-crash-safety
description: "Use when writing any non-test Rust in Chamfera, handling input from files, users, scripts or agents, or investigating a crash or hang."
---

# Never crash, never corrupt

A CAD app that crashes loses someone's work. The workspace lints deny `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` and `dbg!` outside tests, and `unsafe` everywhere.

## Rules
- Errors are `Result` with a typed error (`thiserror`). Messages say what was wrong and what is allowed: "`value` must be a finite number", not "invalid".
- All input is hostile: command parameters, design files, imported files, script files, MCP calls. Validate before acting: types, finite numbers, ranges (`chamfera_geom::MAX_COORDINATE`), name rules, sizes (cap file sizes and counts).
- A failing operation leaves the design unchanged. `Session::execute` restores the snapshot for `mutates` commands; kernel operations return a whole new body or an error, never a half-built one.
- Floating point: compare with tolerances (`LINEAR_TOLERANCE`, `ANGULAR_TOLERANCE`), never `==` on computed values. Guard divisions and `normalized()` (it returns `Option`).
- Indexing: prefer `get()` over `[i]` where the index comes from input or topology.
- Recursion and loops over input are bounded (depth, iteration limits); solvers have a maximum iteration count.
- Reading a file re-validates everything in it (`Document::from_json`).

## Tests that guard this
- `hostile_params_never_panic` in `crates/engine/src/tests.rs` runs every command with hostile parameters. A new parameter kind gets a hostile case there.
- A crash fix comes with a test in the area's existing test file that reproduces it.
