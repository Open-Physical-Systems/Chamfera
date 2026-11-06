# Chamfera: instructions for agents

Chamfera is an open-source, from-scratch parametric 3D CAD application in Rust: our own sketch solver, feature timeline and B-rep kernel, with a Fusion-style workflow. CAM comes later. This file is a router: the hard rules, then the skill to read for your task.

## Start here
Status and the next milestone: `PLAN.md`. Layers: `docs/architecture.md`. Kernel order: `docs/kernel-ladder.md`.

## Hard rules (and what enforces each)
- Independent: never copy another product's code, icons, text, templates or file internals; never drive commercial CAD for test data. `skills/chamfera-ground-rules/SKILL.md`.
- No GPL/LGPL/AGPL/MPL code, and do not even read it (OpenCASCADE, FreeCAD, SolveSpace, CadQuery, LibreCAD, OpenCAMLib): `cargo xtask licences`.
- The kernel is our own, written from textbooks, papers and public specifications: `skills/chamfera-kernel/SKILL.md`.
- Never crash: no unwrap/expect/panic/todo/unsafe in non-test code; errors are `Result`; all input is hostile: clippy `-D warnings`, `hostile_params_never_panic`.
- Model decisions use exact geometry; meshes are for display and export only.
- Everything is a command, and programmatic calls never open dialogs: `skills/chamfera-commands/SKILL.md`.
- Layering: geom (0) -> sketch, kernel, render (1) -> doc (2) -> io (3) -> engine (4) -> gpu (5) -> ui, mcp (6) -> apps (7). Nothing below layer 5 uses egui, winit, wgpu or rfd: `cargo xtask layers`.
- Colours come from the theme tokens in `crates/render/src/theme.rs`; camera input goes only through `view.*` commands: `skills/chamfera-ui/SKILL.md`.
- Show visible changes with a real screenshot you captured (`chamfera --screenshot`, `chamfera-cli snapshot`) and looked at: `skills/chamfera-agent-tools/SKILL.md`.
- Libraries below the UI build for wasm: `cargo xtask wasm`.
- Units are millimetres and radians internally; Z is up.
- The product name is Chamfera; crates are `chamfera-*`, design files `.chamfera`.
- Code, comments, commit messages and docs are in English.
- A test measures a result against a value derived by hand or analytically, never against "whatever the code returned": `skills/chamfera-tests/SKILL.md`.
- Independent work runs in parallel (one subagent per unit, no two writing the same file); dependent work stays sequential.
- Branches: `dev` is the default development branch; work on a topic branch from `dev` and merge back into `dev`. `main` only receives releases from `dev`, and each release is tagged. Never commit directly to `main`: `skills/chamfera-contributing/SKILL.md`.
- Run `cargo xtask ci` before every commit. `cargo xtask skills` keeps this file and `skills/` honest.

## Working on X: read the skill first
| Working on | Read |
|---|---|
| Commits, branches, the workflow, what to work on | `skills/chamfera-contributing/SKILL.md` |
| Licences, dependencies, assets, the name | `skills/chamfera-ground-rules/SKILL.md` |
| Non-test Rust, hostile input, a crash | `skills/chamfera-crash-safety/SKILL.md` |
| A command, a crate boundary | `skills/chamfera-commands/SKILL.md` |
| Writing or running tests | `skills/chamfera-tests/SKILL.md` |
| Geometry, topology, the B-rep kernel, the sketch solver | `skills/chamfera-kernel/SKILL.md` |
| A panel, menu, the viewport, colours, camera input | `skills/chamfera-ui/SKILL.md` |
| Running the app, screenshots, snapshots | `skills/chamfera-agent-tools/SKILL.md` |
| Building a part as an agent over MCP | `skills/chamfera-modeling/SKILL.md` |
| The AppImage, releases, packages | `skills/chamfera-packaging/SKILL.md` |
