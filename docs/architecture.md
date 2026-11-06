# Architecture

| Layer | Crates | Role |
|---|---|---|
| L0 | `geom` | vectors, points, tolerances; later curves, surfaces, transforms, meshes |
| L1 | `sketch`, `kernel`, `render` | the constraint solver and profiles; our own B-rep kernel; camera math and the headless scene |
| L2 | `doc` | parameters, expressions, the feature timeline, persistent names, the design format |
| L3 | `io`, `drawing`, `cam`, `raytrace` | design files; STEP, STL, 3MF import and export. 2D drawings: projected views with hidden lines from the exact B-rep, sheets, dimensions, SVG/DXF/PDF export. CAM: 2.5D milling toolpaths (facing, contour, pocket, drilling) from the exact model, and G-code for LinuxCNC and GRBL. Raytrace: a CPU path tracer for presentation images of the display meshes |
| L4 | `engine` | the session, undo, and the command registry (everything is a command) |
| L5 | `gpu`, `assistant` | the wgpu viewport renderer, and windowless snapshots of the same picture; the in-app Claude assistant (Messages API client, streaming, the tool loop; no egui) |
| L6 | `ui`, `mcp` | the egui front end for desktop and browser; the MCP server for agents over stdio (`docs/mcp.md`) |
| L7 | `chamfera`, `chamfera-cli`, `chamfera-web` | desktop app, headless CLI, browser app |

A crate depends only on lower layers (`cargo xtask layers`). Nothing below L5 uses window, GPU or dialog crates, so the core builds for wasm (`cargo xtask wasm`) and runs headless.

## The viewport
`render` (L1) holds everything the picture depends on as plain data: the camera, the scene of bodies, the selection and the theme colours. `gpu` (L5) draws that into its own texture; the UI shows the texture, and `chamfera-cli snapshot` draws the same frame without a window. The UI never moves the camera itself: mouse, touchpad and keys become `view.*` and `select.*` commands (`navigate` in `crates/ui/src/lib.rs`).

## The Assistant
`chamfera-assistant` (L5, beside `gpu`, depending only on `engine`) lets Claude build a design live from the Assistant panel (`crates/ui/src/assistant_ui.rs`, View > Assistant, Ctrl+Shift+A). Every registry command except the file ones becomes a Claude tool (`solid.extrude` is `solid_extrude`, its input schema read from the parameter docs), plus `design_summary` and `screenshot`; `chamfera_engine::agent` holds what it shares with the MCP server (the file-command list, the design summary with exact volumes, base64). The Messages API is called over HTTPS (`ureq` + `rustls`, desktop only) on a worker thread; each tool call is sent to the UI thread over a channel and runs through `Session::execute`, so the viewport updates after each step. Destructive and file commands wait for Allow in the panel; Stop cancels between steps; a request's steps are merged into one undo step (`Session::group_undo_since`). The key lives in `assistant-key` (0600) beside the preferences file, or comes from `ANTHROPIC_API_KEY`; never in a design. The browser build shows a desktop-only note. `chamfera --assistant-demo` plays a recorded conversation for screenshots.

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

## Rendering
Materials are part of `body.appearance` (`crates/doc/src/appearance.rs`, format 34: metallic, roughness, transmission, IOR); the library is `crates/render/src/material.rs` and the lighting presets (sky gradient, key, fill and rim disc lights, a floor) `crates/render/src/environment.rs`, so the viewport's realistic style (`crates/gpu/src/realistic.rs`, `realistic.wgsl`: GGX, a key-light shadow map) and the path tracer (`chamfera-raytrace`, L3: BVH, Lambert + GGX, smooth dielectric, MIS towards the lights, progressive on std threads) show the same scene. The environment is view state (`render.environment`). Commands: `render.*` (`crates/engine/src/cmd/render.rs`); the session holds the last started render (`Session::render_job`), which the RENDER workspace (`crates/ui/src/render_ui.rs`) steps on a worker thread and shows as it refines.

## Assemblies
Components and joints are stored in the design (`crates/doc/src/component.rs`, `joint.rs`, format 15) and solved as an exact chain from the grounded components (`crates/engine/src/assembly.rs`). Motion study keys (`crates/doc/src/motion.rs`, format 27) are joint values at times, interpolated linearly. What a joint drag, a motion time and an exploded view show is display state in the session (`crates/engine/src/motion.rs`, `AssemblyView`): never part of the design or of undo; an exploded view moves the shown meshes, not the exact bodies. Interference intersects the placed exact bodies (K7) for pairs whose boxes overlap. UI: `crates/ui/src/assembly_ui.rs` and `crates/ui/src/motion_ui.rs`.

## Manufacture (CAM)
The design's `cam` section (`crates/doc/src/cam.rs`, format 20) stores setups (stock, part zero), the tool library and operations, which name model faces and edges by their persistent names. `chamfera-cam` (L3) reads boundaries, pocket regions and holes from the B-rep, offsets lines and arcs exactly (offset, split, trim by distance, join), and makes toolpaths and G-code. The engine works the toolpaths out again after every rebuild (`crates/engine/src/cam.rs`) and draws them as overlays: feeds solid, rapids dashed, in the `toolpath_*` theme colours. Commands: `cam.*` (`crates/engine/src/cmd/cam.rs`); UI: the MANUFACTURE tab (`crates/ui/src/cam_ui.rs`). Refused for now: undercuts, walls that are not vertical, open chains, ellipses, holes not along Z.
