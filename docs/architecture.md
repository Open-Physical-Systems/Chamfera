# Architecture

| Layer | Crates | Role |
|---|---|---|
| L0 | `geom` | vectors, points, tolerances; later curves, surfaces, transforms, meshes |
| L1 | `sketch`, `kernel`, `render` | the constraint solver and profiles; our own B-rep kernel; camera math and the headless scene |
| L2 | `doc` | parameters, expressions, the feature timeline, persistent names, the design format |
| L3 | `io` | design files; STEP, STL, 3MF import and export |
| L4 | `engine` | the session, undo, and the command registry (everything is a command) |
| L5 | `ui` (M1), `mcp` (M6) | the egui + wgpu front end; the MCP server for agents |
| apps | `chamfera-cli`, `chamfera` (M1), `chamfera-web` (M1) | headless CLI, desktop app, browser app |

A crate depends only on lower layers (`cargo xtask layers`). Nothing below L5 uses window, GPU or dialog crates, so the core builds for wasm (`cargo xtask wasm`) and runs headless.

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
