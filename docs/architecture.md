# Architecture

| Layer | Crates | Role |
|---|---|---|
| L0 | `geom` | vectors, points, tolerances; later curves, surfaces, transforms, meshes |
| L1 | `sketch`, `kernel`, `render` | the constraint solver and profiles; our own B-rep kernel; camera math and the headless scene |
| L2 | `doc` | parameters, expressions, the feature timeline, persistent names, the design format |
| L3 | `io`, `cam` | design files; STEP, STL, 3MF import and export. CAM: 2.5D milling toolpaths (facing, contour, pocket, drilling) from the exact model, and G-code for LinuxCNC and GRBL |
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

## Manufacture (CAM)
The design's `cam` section (`crates/doc/src/cam.rs`, format 20) stores setups (stock, part zero), the tool library and operations, which name model faces and edges by their persistent names. `chamfera-cam` (L3) reads boundaries, pocket regions and holes from the B-rep, offsets lines and arcs exactly (offset, split, trim by distance, join), and makes toolpaths and G-code. The engine works the toolpaths out again after every rebuild (`crates/engine/src/cam.rs`) and draws them as overlays: feeds solid, rapids dashed, in the `toolpath_*` theme colours. Commands: `cam.*` (`crates/engine/src/cmd/cam.rs`); UI: the MANUFACTURE tab (`crates/ui/src/cam_ui.rs`). Refused for now: undercuts, walls that are not vertical, open chains, ellipses, holes not along Z.
