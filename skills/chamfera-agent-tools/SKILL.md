---
name: chamfera-agent-tools
description: "Use when running Chamfera to see a change working: the desktop app, window screenshots, windowless snapshots of the viewport, and driving the CLI with commands or scripts."
---

# Running Chamfera and looking at it

## The desktop app
- `cargo run -p chamfera -- --sample` opens the window with the sample bracket; give `.stl` paths to insert meshes; `--theme light|dark` forces a theme.
- `cargo run -p chamfera -- --sample --screenshot /path/shot.png` opens the window, waits for it to draw, saves a screenshot of the whole window and quits. Then read the PNG and look at it.
- `--script steps.json` runs commands at start (same format as `chamfera-cli run`), so a screenshot can show a state reached by commands, for example a sketch being edited (`examples/sketch-plate.json`).
- More start-up options for screenshots: `--size WxH` (window size), `--zoom F` (UI scale, e.g. 2 for a HiDPI check), `--extrude` (opens the Extrude tool with the profile under the viewport centre picked), `--search "ext"` (opens the search palette with that query typed).
- Scripts can name a step's reply with `"as": "axis"` and use it later as `"$axis.curve"` (`crates/engine/src/script.rs`), in the CLI and in `--script`.
- It needs a display (Wayland or X11). Without one, use the snapshot below.

## Snapshots without a window
`cargo run -p chamfera-cli -- snapshot --sample --out shot.png` renders the viewport with the same renderer the app uses. Options:
- `--stl part.stl` (repeatable) and `--script steps.json`;
- `--view iso|front|back|left|right|top|bottom`;
- `--projection perspective|orthographic`;
- `--theme light|dark`;
- `--size 1280x800`;
- `--pick X,Y`, which prints what that pixel hits and shows it selected.

It uses the GPU if there is one, else a software Vulkan driver (lavapipe, `mesa-vulkan-drivers`).

## Sketches
- `sketch.create {"plane": "xy"}` makes a sketch active; `sketch.line`, `sketch.rectangle`, `sketch.circle`, `sketch.arc` draw; `sketch.constrain {"constraint": {"kind": ..., ...}}` constrains; `sketch.get` returns points, curves, constraints, DOF, status and profile areas. Check DOF and areas against values you worked out.
- Parameters: `param.set {"name": "width", "expression": "80 mm"}`; a dimension follows one with `sketch.dimension {"constraint": id, "expression": "width / 2"}` (`examples/parametric-plate.json`). `chamfera --parameters` opens the Parameters window at start.
- Points, curves and constraints share one id counter per sketch: read ids from each reply instead of guessing them.
- Sketch tools: `sketch.trim {"curve", "x", "y"}`, `sketch.extend {"curve", "x", "y"}`, `sketch.split {}` (every crossing; do this before extruding overlapping shapes), `sketch.fillet {"point", "radius"}`, `sketch.chamfer {"point", "distance"}`, `sketch.offset {"curves": [id], "distance", "corners": "sharp"|"round"}`, `sketch.mirror {"axis", "curves"}`, `sketch.pattern.rectangular`, `sketch.pattern.circular`, `sketch.rectangle.centre`, `sketch.arc.three_point`, `sketch.arc.tangent`. Each replies with `created_curves`, `changed_curves`, `removed_curves` and the points and constraints, so a script can name the reply (`"as": "r"`) and use `"$r.created_curves.0"`.

## Manufacture (CAM)
- `cam.setup`, `cam.tool_add`, `cam.op_add {"kind": "facing"|"contour"|"pocket"|"drill", "tool", ...}`, `cam.op_edit`, `cam.generate`, `cam.list`, `cam.post {"dialect": "linuxcnc"|"grbl", "path"}` (without `path` the G-code comes back as `gcode`). Faces are named like `4:top` (feature id, role); `examples/hole-plate.json` builds a part and all four operations.
- Check `cam.generate`'s reply (lengths, minutes, per-operation `error`) against values you worked out; a script that runs `cam.generate` opens the app in the MANUFACTURE tab with the toolpaths drawn.

## Commands and scripts
- `chamfera-cli commands` lists every command; `chamfera-cli exec <id> '<json>'` runs one.
- `chamfera-cli run script.json` runs a list of `{"command", "params"}` steps and prints one reply per line (`examples/parameters.json`).
- Check results with the numbers in the replies (volume, bounds, pick point and normal) against values you worked out, not only with pictures.

## Evidence
For a visible change, attach a screenshot or snapshot you captured and looked at, with before and after when it changes something that existed.
