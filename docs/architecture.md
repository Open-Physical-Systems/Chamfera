# Architecture

| Layer | Crates | Role |
|---|---|---|
| L0 | `geom` | vectors, points, tolerances; later curves, surfaces, transforms, meshes |
| L1 | `sketch`, `kernel`, `render` | the constraint solver and profiles; our own B-rep kernel; camera math and the headless scene |
| L2 | `doc` | parameters, expressions, the feature timeline, persistent names, the design format |
| L3 | `io`, `drawing` | design files; STEP, STL, 3MF import and export. 2D drawings: projected views with hidden lines from the exact B-rep, sheets, dimensions, SVG/DXF/PDF export |
| L4 | `engine` | the session, undo, and the command registry (everything is a command) |
| L5 | `gpu` | the wgpu viewport renderer, and windowless snapshots of the same picture |
| L6 | `ui`, `mcp` | the egui front end for desktop and browser; the MCP server for agents over stdio (`docs/mcp.md`) |
| L7 | `chamfera`, `chamfera-cli`, `chamfera-web` | desktop app, headless CLI, browser app |

A crate depends only on lower layers (`cargo xtask layers`). Nothing below L5 uses window, GPU or dialog crates, so the core builds for wasm (`cargo xtask wasm`) and runs headless.

## The viewport
`render` (L1) holds everything the picture depends on as plain data: the camera, the scene of bodies, the selection and the theme colours. `gpu` (L5) draws that into its own texture; the UI shows the texture, and `chamfera-cli snapshot` draws the same frame without a window. The UI never moves the camera itself: mouse, touchpad and keys become `view.*` and `select.*` commands (`navigate` in `crates/ui/src/lib.rs`).

## Data flow
```
UI click / CLI / script / MCP
        |  command id + JSON params
        v
engine::Session::execute ---- snapshot (undo) / rollback on error
        |
        v
doc::Document (parameters, timeline) --recompute--> sketch solver + kernel ops --> bodies
                                                                       |
                                                    tessellate (display/export only)
```
