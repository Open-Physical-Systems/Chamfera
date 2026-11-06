---
name: chamfera-agent-tools
description: "Use when running Chamfera to see a change working: the desktop app, window screenshots, windowless snapshots of the viewport, and driving the CLI with commands or scripts."
---

# Running Chamfera and looking at it

## The desktop app
- `cargo run -p chamfera -- --sample` opens the window with the sample bracket; give `.stl` paths to insert meshes; `--theme light|dark` forces a theme.
- `cargo run -p chamfera -- --sample --screenshot /path/shot.png` opens the window, waits for it to draw, saves a screenshot of the whole window and quits. Then read the PNG and look at it.
- `--script steps.json` runs commands at start (same format as `chamfera-cli run`), so a screenshot can show a state reached by commands, for example a sketch being edited (`examples/sketch-plate.json`).
- More start-up options for screenshots: `--size WxH` (window size), `--zoom F` (UI scale, e.g. 2 for a HiDPI check), `--extrude` (opens the Extrude tool with the profile under the viewport centre picked), `--extrude-with '{"taper": 5, "direction": "two_sides", "distance2": 4}'` (the same with those `solid.extrude` options set, and its live preview shown), `--box X1,Y1,X2,Y2` (shows a selection box at those viewport pixels as if being dragged; pair it with a `select.box` step, as in `examples/box-select.json`), `--search "ext"` (opens the search palette with that query typed), `--context-menu X,Y` (opens the viewport right-click menu at that viewport point, for the body, face, edge, empty-space or sketch menu), `--hole` (opens the Hole tool on the last hole feature, e.g. after `examples/hole-plate.json`), `--preferences general|navigation|units|grid` (opens the Preferences window on that page), `--press-pull X,Y,D` (opens Press Pull on what is under that viewport point and sets the distance to D mm as a drag would, with its arrow and live preview).
- `--sketch-input "tool line; click 0,0; hover 30,12; shift; type 25; tab"` drives the drawing tools in the active sketch after start (points in sketch mm), so a screenshot can show a snap marker, the Shift lock or the typed-value fields (`examples/sketch-snaps.json`).
- Scripts can name a step's reply with `"as": "axis"` and use it later as `"$axis.curve"` (`crates/engine/src/script.rs`), in the CLI and in `--script`.
- It needs a display (Wayland or X11). Without one, use the snapshot below.

## Drawings
- `drawing.new_sheet`, `drawing.add_view {view, x, y}`, `drawing.add_dimension {view, kind, x1, y1, x2, y2}` (sheet mm, snapped to projected vertices and edges) and `drawing.export {path}` (svg, dxf or pdf); `drawing.list` gives each view's place, box and line counts and each dimension's value.
- `chamfera --script examples/drawing-sheet.json --drawing --screenshot shot.png` shows the sheet in the DRAWING workspace. An exported SVG can be looked at with `google-chrome --headless --screenshot`, a PDF with `pdftoppm -png`.

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
- Splines: `sketch.spline {"points": [{"x", "y"} or {"point": id}], "kind": "fit"|"control", "closed"}` (`examples/spline-extrude.json`); the points are ordinary sketch points, so constrain or drag them to shape the spline.
- Sketch tools: `sketch.trim {"curve", "x", "y"}`, `sketch.extend {"curve", "x", "y"}`, `sketch.split {}` (every crossing; do this before extruding overlapping shapes), `sketch.fillet {"point", "radius"}`, `sketch.chamfer {"point", "distance"}`, `sketch.offset {"curves": [id], "distance", "corners": "sharp"|"round"}`, `sketch.mirror {"axis", "curves"}`, `sketch.pattern.rectangular`, `sketch.pattern.circular`, `sketch.rectangle.centre`, `sketch.arc.three_point`, `sketch.arc.tangent`. Each replies with `created_curves`, `changed_curves`, `removed_curves` and the points and constraints, so a script can name the reply (`"as": "r"`) and use `"$r.created_curves.0"`.
- Shapes, which reply the same way: `sketch.polygon {"cx", "cy", "x", "y", "sides", "mode": "inscribed"|"circumscribed"}`, `sketch.slot {"x1", "y1", "x2", "y2", "width", "overall"}`, `sketch.ellipse {"cx", "cy", "x", "y", "minor"}`, `sketch.ellipse.arc` (plus start and end directions `x1, y1, x2, y2`).
- `sketch.import_dxf {"path", "units": "mm"|"inch", "scale"}` draws a 2D DXF into the active sketch (units default to the file's `$INSUNITS`) and replies with the curves, joined endpoints, skipped entities and profile areas (`examples/dxf-import.json`); `sketch.export_dxf {"path"}` writes the sketch back out.
- `sketch.project {"edges": ["2:top 0"]}` or `{"faces": [{"feature": 2, "face": "top"}]}` copies model edges into the active sketch as fixed curves that follow the model when it rebuilds (`"linked": false` for a plain copy); `examples/sketch-shapes.json` shows both.

## Construction geometry
- `construct.plane_offset {"base": "xy" | {"feature", "face"} | plane id, "distance"}`, `construct.plane_angle`, `construct.plane_midplane`, `construct.plane_three_points`, `construct.plane_tangent`, `construct.axis_cylinder`, `construct.axis_edge`, `construct.axis_two_points`, `construct.axis_two_planes`, `construct.point_vertex`, `construct.point_centre`, `construct.point_coordinates`; each replies with its `geometry` (origin, normal, direction or point) to check against hand values.
- `construct.pick {x, y, width, height}` names the face, nearest edge and vertex under a pixel in the form these commands take.
- `sketch.create {"plane": <plane id>}` sketches on a construction plane and follows it; `solid.revolve` and `solid.pattern_circular` take `axis_feature` (`examples/construction-planes.json`).

## Sheet metal
- `sheet.rule {name, thickness, bend_radius, k_factor, relief, relief_width, relief_depth}` makes or changes a named rule; `sheet.base {sketch, profile: {x, y}, rule}` thickens a closed profile; `sheet.flange {edge, height, angle, position, side, offset_start, offset_end}` bends a wall up from a straight edge (names from `solid.edges`); `sheet.flat_pattern` unfolds it (replies with `length`, `width`, `area` and each bend's `allowance`) and `sheet.export_flat {path}` writes the outline as DXF. `examples/sheet-bracket.json` builds a bracket and its flat pattern.
## Manufacture (CAM)
- `cam.setup`, `cam.tool_add`, `cam.op_add {"kind": "facing"|"contour"|"pocket"|"drill", "tool", ...}`, `cam.op_edit`, `cam.generate`, `cam.list`, `cam.post {"dialect": "linuxcnc"|"grbl", "path"}` (without `path` the G-code comes back as `gcode`). Faces are named like `4:top` (feature id, role); `examples/cam-plate.json` builds a pocketed plate and all four operations.
- Check `cam.generate`'s reply (lengths, minutes, per-operation `error`) against values you worked out; a script that runs `cam.generate` opens the app in the MANUFACTURE tab with the toolpaths drawn.

## Commands and scripts
- `chamfera-cli commands` lists every command; `chamfera-cli exec <id> '<json>'` runs one.
- `chamfera-cli run script.json` runs a list of `{"command", "params"}` steps and prints one reply per line (`examples/parameters.json`).
- Measure: `inspect.measure {"a": {"solid": "Extrude1", "face": "2:top"}, "b": {"point": [x, y, z]}}` gives the distance, both closest points and the angle; entities are `{solid}`, `{solid, face}`, `{solid, edge}` (names from `solid.edges`), `{solid, vertex}` or `{point}`; `inspect.measure_at {x, y, width, height, from?}` picks by pixel; `inspect.mass {"body": "Extrude1", "density": 1}` gives volume, mass, centre of mass and inertia. A `--script` whose last inspect step is `inspect.measure` or `inspect.mass` opens that tool with its result, so a screenshot shows it (`examples/measure-block-and-pin.json`).
- Check results with the numbers in the replies (volume, bounds, pick point and normal) against values you worked out, not only with pictures.

## Evidence
For a visible change, attach a screenshot or snapshot you captured and looked at, with before and after when it changes something that existed.
