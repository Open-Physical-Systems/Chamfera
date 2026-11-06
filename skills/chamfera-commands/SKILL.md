---
name: chamfera-commands
description: "Use when adding or changing a Chamfera command or any user-visible behaviour, or when deciding which crate code belongs in. Covers CommandSpec, command ids, undo, and layering."
---

# Adding a command

Everything a user can do is a command. The UI (M1), `chamfera-cli`, scripts and the MCP server (M6) all call `Session::execute(id, params)`.

## Steps
1. Pick the area file in `crates/engine/src/cmd/` (or add one, and add it to `registry()` in `crates/engine/src/cmd/mod.rs`, keeping the `mod` lines sorted).
2. Add a `CommandSpec`: `id`, `label`, `params` (one line per parameter: `name: type (required|optional) - meaning`), `mutates`, `run`. Keep each area's list sorted by id.
3. The id is our own: dotted, lower-case `<area>.<thing>[.<variant>]`, such as `solid.extrude` or `sketch.rectangle.two_point`. Never another product's id.
4. Read parameters only through `crates/engine/src/params.rs` helpers; they validate. Range-check everything else before changing the design.
5. Set `mutates: true` if the command can change the design. The session snapshots first, rolls back on error and records one undo step on success.
6. Test it in `crates/engine/src/tests.rs`: the result measured against a hand-derived value, the failure cases, and undo. Add hostile cases for new parameter names to `hostile_params_never_panic`.

## Programmatic calls never open dialogs
A dialog (M1+) is a front-end form that builds the parameters and then runs the command. `execute` never waits for a user.

## Layering
`cargo xtask layers` fails when a crate depends on its own or a higher layer:
geom (0) -> sketch, kernel, render (1) -> doc (2) -> io, drawing (3) -> engine (4) -> gpu (5) -> ui, mcp (6) -> apps (7).
- Geometry and topology go in `geom` or `kernel`; the solver in `sketch`; what a design stores in `doc`; files in `io`; commands in `engine`. The UI only reads engine state and runs commands.
- A new crate needs a row in `LAYERS` in `xtask/src/main.rs` and in `docs/architecture.md`.
