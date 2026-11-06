# Feature backlog: parity with Fusion-style 3D CAD and AutoCAD-style 2D drafting

*Written 2026-11-06 from `PLAN.md`, `docs/kernel-ladder.md`, the output of `chamfera-cli commands` and the unmerged branches `sketch/tools`, `engine/blends` and `web/browser-app`. Every description is our own wording of what a user of these products expects; nothing here is taken from their documentation, and the names are generic (see `skills/chamfera-ground-rules/SKILL.md`).*

**Status:** `done` = on `dev`; `branch` = on an unmerged branch (named); `missing` = not started (a note says when groundwork exists).
**Priority:** P0 = a Fusion user expects it on day one; P1 = important; P2 = later.
**Size:** S = days; M = one to three weeks; L = a month or more, or a kernel rung.
**Layer:** the crate(s) touched: `geom`, `sketch`, `kernel`, `render`, `doc`, `io`, `engine`, `gpu`, `ui`, `mcp`, `apps`; plus `cam` (a planned L1 crate).

## Next 10 to build

Step 0, before anything else: merge `sketch/tools`, `engine/blends` and `web/browser-app` into `dev`, so the next units start from the same base.

Items 1 to 7 can run in parallel (they touch different crates or files); 8 to 10 depend on earlier ones.

| # | Feature | Why now | Size | Main crates | Depends on |
|---|---|---|---|---|---|
| 1 | **Shell** (hollow a solid, remove chosen faces, wall thickness) | The third most used solid feature after extrude and fillet; kernel rung K12 | L | kernel, engine | none |
| 2 | **Measure and mass properties** (distance, angle, edge length, face area, volume, centre of mass) | Users and agents check every part with it; the kernel already computes exact area and volume (done) | M | kernel, engine, ui | none |
| 3 | **Construction planes, axes and points** (offset, at angle, midplane, through three points, tangent; axis through edge/two points; points) | Needed to sketch anywhere other than XY/XZ/YZ or a flat face | M | doc, engine, ui | none |
| 4 | **Command search and keyboard shortcuts** (search box listing every command; single-key shortcuts for the common tools; right-click context menus in the viewport) | Every command already exists in the registry, so this is mostly UI | M | ui | none |
| 5 | **Typed values and richer inference while drawing** (type length and angle in fields beside the cursor; snaps to midpoint, centre, quadrant, on curve, tangent, perpendicular; Shift to suspend snaps) | The sketcher feels unfinished without it; AutoCAD and Fusion users both rely on it | M | ui, engine (sketch commands) | none |
| 6 | **More sketch entities: polygon, slot, ellipse, project/include geometry** (project edges, faces and other sketches onto the sketch plane, linked) | Projection is how a second feature lines up with the first | M | sketch, engine, ui | none (projection uses the face sketch work) |
| 7 | **Extrude extents and options** (through all, to a face or body, start offset, two sides with different distances, taper angle, new body / join / cut / intersect chosen in the dialog) | Most real extrudes use one of these | M | engine, doc, kernel (taper adds cone and inclined plane sides) | none |
| 8 | **Hole feature** (simple, counterbore, countersink, blind or through, placed on a face at sketch points; thread shown as a cosmetic attribute) | Day-one feature for mechanical parts | M | engine, doc, kernel | 3 (points), 7 (through all) |
| 9 | **Mirror and move/copy of bodies and features** (mirror across a plane; translate and rotate bodies, with a copy option) | Basic editing; patterns are done so the transform path exists | M | engine, doc, kernel | 3 (planes to mirror across) |
| 10 | **Persistent naming (K8) and the rollback bar** (names survive any edit; drag a marker in the timeline to roll the model back and insert features there) | Without K8, editing early features breaks later ones; the rollback bar needs stable references | L | doc, kernel, ui | best after 1 to 9 land, since it touches every feature |

Close runners-up: selection filters and box selection (UI and NAV), splines (needs K10), sweep and loft (K12 and K10), components (ASM).

---

## 1. Sketch entities and tools

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Sketch on origin planes | Start a sketch on XY, XZ or YZ | done | P0 | S | doc, engine, ui |
| Sketch on a flat face | Sketch on a planar face; the sketch follows the face on rebuild | done | P0 | M | doc, engine |
| Sketch on a construction plane | Sketch on any user-made plane | missing | P0 | S | doc, engine |
| Line (chained) | Click-click line chains that close on the first point | done | P0 | S | sketch, ui |
| Two-point rectangle | Rectangle from two corners | done | P0 | S | sketch, ui |
| Centre rectangle | Rectangle from centre and a corner | branch (`sketch/tools`) | P0 | S | sketch, ui |
| Three-point rectangle | Rectangle at any angle from three points | missing | P1 | S | sketch, ui |
| Centre-diameter circle | Circle from centre and radius | done | P0 | S | sketch, ui |
| Two-point, three-point and tangent circles | Circle through points, or tangent to two or three curves | missing | P1 | S | sketch, ui |
| Centre-point arc | Arc from centre, start, end | done | P0 | S | sketch, ui |
| Three-point arc | Arc through start, a middle point, end | branch (`sketch/tools`) | P0 | S | sketch, ui |
| Tangent arc | Arc that continues tangentially from a line end | branch (`sketch/tools`) | P0 | S | sketch, ui |
| Point | A free sketch point | done | P0 | S | sketch |
| Construction toggle | Mark a curve as construction (not part of profiles) | done | P0 | S | sketch, ui |
| Polygon | Regular polygon, inscribed or circumscribed, by centre or edge | branch (`sketch/shapes-project`) (by centre) | P0 | S | sketch, ui |
| Slot | Straight slot by centres or overall length, arc slot | branch (`sketch/shapes-project`) (straight) | P0 | S | sketch, ui |
| Ellipse | Ellipse by centre and two axes (kernel ellipses exist) | branch (`sketch/shapes-project`) (not extruded yet) | P1 | S | sketch, ui |
| Elliptical arc | Partial ellipse | branch (`sketch/shapes-project`) (command only) | P2 | S | sketch |
| Conic curve | A conic with a shape value (rho) between two ends | missing | P2 | M | sketch, geom |
| Fit-point spline | Smooth curve through clicked points, with tangent handles | branch (`kernel/k10-splines`): `sketch.spline`, Spline tool, open and closed, extrudes; tangent handles and end-tangent constraints not yet | P1 | L | sketch, geom (needs K10) |
| Control-point spline | Spline edited by its control polygon | branch (`kernel/k10-splines`): `sketch.spline {kind: control}`, open and closed; no UI tool of its own yet | P1 | M | sketch, geom (needs K10) |
| Sketch text | Text as closed profiles, with a font, height and alignment on a path | missing | P2 | L | sketch, io (font parsing) |
| Trim | Delete the piece of a curve between crossings | branch (`sketch/tools`) | P0 | M | sketch, engine, ui |
| Extend | Lengthen a curve to the next one it meets | branch (`sketch/tools`) | P0 | S | sketch, engine, ui |
| Split / break | Cut a curve at a point or at its crossings | branch (`sketch/tools`) | P0 | S | sketch, engine |
| Sketch fillet | Round a corner between two curves with a tangent arc | branch (`sketch/tools`) | P0 | S | sketch, engine |
| Sketch chamfer | Bevel a corner between two lines | branch (`sketch/tools`) | P1 | S | sketch, engine |
| Offset | Parallel copy of a chain at a distance | branch (`sketch/tools`) | P0 | M | sketch, engine |
| Sketch mirror | Mirror curves across a line, with symmetry constraints | branch (`sketch/tools`) | P0 | S | sketch, engine |
| Sketch rectangular and circular patterns | Repeat curves in a grid or around a point | branch (`sketch/tools`) | P1 | S | sketch, engine |
| Move, rotate, scale, copy sketch geometry | Transform selected curves by drag or typed values | missing | P1 | M | sketch, engine, ui |
| Project / include geometry | Bring model edges, faces and other sketch curves into the sketch, linked to their source | branch (`sketch/shapes-project`) (edges and faces) | P0 | M | sketch, doc, engine |
| Intersection curve | Curve where the sketch plane cuts a body | missing | P1 | M | kernel, sketch |
| Project to surface / wrap | Project curves onto a curved face | missing | P2 | L | kernel |
| 3D sketch | Curves in space, not on one plane (paths for sweeps) | missing | P2 | L | sketch, doc |
| Profile detection | Closed regions, holes, nested profiles found automatically | done | P0 | M | sketch |
| Splitting curves at crossings for profiles | Overlapping curves form regions without trimming first | missing | P0 | M | sketch |
| Large sketches | Sparse solver for sketches with thousands of entities | missing | P2 | L | sketch |
| Import DXF/SVG into a sketch | Bring 2D drawings in as sketch curves | missing | P1 | M | io, sketch |
| Copy and paste sketch geometry | Clipboard for curves between sketches | missing | P2 | S | engine, ui |

## 2. Sketch constraints and dimensions

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Solver (Levenberg-Marquardt, own QR) | Our own solver for the sketch equations | done | P0 | L | sketch |
| Coincident, point on line, point on circle | Points meet, or lie on a curve | done | P0 | S | sketch |
| Horizontal, vertical | Line along the sketch axes | done | P0 | S | sketch |
| Parallel, perpendicular | Line relationships | done | P0 | S | sketch |
| Tangent | Line-arc and arc-arc tangency | done | P0 | S | sketch |
| Equal | Same length or same radius | done | P0 | S | sketch |
| Fix / lock | Pin a point | done | P0 | S | sketch |
| Midpoint | Point at the middle of a line | done | P0 | S | sketch |
| Symmetric | Two points mirrored about a line | done | P0 | S | sketch |
| Concentric | Two circles or arcs share a centre (now done by coincident centres; a direct tool is expected) | missing | P0 | S | sketch, ui |
| Collinear | Two lines on one infinite line | missing | P0 | S | sketch |
| Horizontal/vertical between two points | Align two points along an axis | missing | P1 | S | sketch |
| Curvature continuity (G2) | Smooth join for splines | missing | P2 | M | sketch (needs splines) |
| Linear dimensions | Distance between points, point to line, line length | done | P0 | S | sketch, ui |
| Radius and diameter dimensions | On circles and arcs | done | P0 | S | sketch, ui |
| Angle dimension | Between two lines | done | P0 | S | sketch, ui |
| Aligned / horizontal / vertical dimension choice | Pick which direction a linear dimension measures by where it is placed | missing | P0 | S | sketch, ui |
| Arc length dimension | Length along an arc | missing | P1 | S | sketch |
| Driven (reference) dimensions | A dimension that shows a value without constraining it | missing | P1 | S | sketch, doc |
| Dimensions as expressions | Type `width/2` or a parameter name into a dimension | done | P0 | S | doc, sketch |
| Inline dimension editing | Edit a value in place on the canvas | done | P0 | S | ui |
| DOF analysis and colour feedback | Under, fully and over-constrained shown; redundant and conflicting constraints reported | done | P0 | M | sketch, ui |
| Constraint glyphs on canvas | Small markers showing each constraint; click to select or delete | missing | P0 | M | ui, render |
| Automatic constraints while drawing | Horizontal/vertical, coincident, tangent, perpendicular added as you draw (H/V and coincident done) | missing | P0 | M | engine, ui |
| Auto-constrain / auto-dimension | Fully constrain a sketch in one step | missing | P2 | M | sketch |
| Drag with constraints | Drag a point; the sketch follows its constraints | done | P0 | S | sketch, ui |
| Show unconstrained geometry | Highlight what can still move | done | P0 | S | ui |
| Conflict dialog | When a new constraint conflicts, list the constraints involved and offer to remove one | missing | P1 | S | ui |

## 3. Drawing input: snaps, inference, ortho and typed values

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Snap to existing points | Clicks near a point reuse it | done | P0 | S | engine, ui |
| Horizontal/vertical inference | Lines within a small angle become horizontal or vertical | done | P0 | S | engine |
| Endpoint, midpoint, centre, quadrant snaps | The usual object snaps, shown with a marker as the cursor nears them | missing | P0 | M | ui, engine |
| On-curve, intersection, tangent, perpendicular, nearest snaps | Snaps to points on or between curves | missing | P0 | M | ui, engine, sketch |
| Snap settings | Choose which snaps are active, and a snap radius in pixels | missing | P1 | S | ui |
| Alignment (tracking) lines | Dashed guide when the cursor lines up with another point horizontally or vertically | missing | P1 | M | ui |
| Polar tracking | Guides at a chosen angle step (for example every 15 degrees) | missing | P1 | S | ui |
| Ortho mode | Force lines horizontal or vertical while held or toggled | missing | P1 | S | ui |
| Hold a modifier to suspend snapping | Draw freely without inference | missing | P0 | S | ui |
| Typed length and angle while drawing | Fields next to the cursor; Tab moves between them; Enter places | missing | P0 | M | ui |
| Typed coordinates | Absolute, relative and polar coordinate entry, AutoCAD style | missing | P1 | M | ui, engine |
| Sketch grid snap | Snap to a visible sketch grid with a chosen spacing | missing | P1 | S | ui, render |
| Rubber-band preview | Show the curve being drawn before the click | done | P0 | S | ui |
| Dimensions shown while drawing | Live length/angle readout | missing | P0 | S | ui |

## 4. Solid features

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Extrude (one side, flipped, symmetric) | Sweep a profile along its normal; distance as an expression | done | P0 | M | kernel, doc, engine |
| Extrude extents | Through all, to a face or object, start offset, two different sides | done (to object: parallel flat faces and planes only) | P0 | M | engine, doc, kernel |
| Extrude taper | Draft angle on the extrude sides | done (planar profiles; sharp corners at arcs refused) | P0 | M | kernel |
| Thin extrude | Extrude an open or closed profile as a wall of given thickness | missing | P1 | M | kernel, engine |
| Revolve | Profile around an axis, partial or full | done | P0 | M | kernel, engine |
| Boolean operation choice | New body, join, cut, intersect for each feature | done | P0 | M | kernel (K7), engine |
| Combine bodies | Boolean between existing bodies, keep tools option | missing | P0 | S | engine |
| Fillet (constant radius) | Round straight edges between planes, and hole/boss rims | branch (`engine/blends`) | P0 | L | kernel (K11), engine, ui |
| Chamfer (equal distance) | Bevel the same edges | branch (`engine/blends`) | P0 | M | kernel (K11), engine |
| Chamfer: two distances, distance and angle | Unequal bevels | missing | P1 | M | kernel |
| Fillet on curved faces in general | Rolling-ball blends between any analytic or NURBS faces | missing | P1 | L | kernel (K11, K10) |
| Variable-radius fillet | Radius changes along the edge | missing | P2 | L | kernel |
| Rule fillet / full round | Round all edges of a face, or a full round between three faces | missing | P2 | M | kernel |
| Tangent chain selection | Picking one edge selects its tangent-connected chain | missing | P0 | S | engine, ui |
| Shell | Hollow a body with chosen faces removed | branch `kernel/k12-shell` (planes, cylinders, cones; inward or outward; closed hollow) | P0 | L | kernel (K12), engine |
| Hole | Simple, counterbore, countersink; blind, to a face, through all | branch (`solid/hole`: simple, counterbore, countersink, blind or through all, drill point; not yet "to a face") | P0 | M | engine, doc, kernel |
| Thread | Cosmetic thread (attribute + drawing note), later modelled thread | missing | P1 | M | doc, kernel |
| Linear (rectangular) pattern | Features repeated in one or two directions | done | P0 | M | engine, doc |
| Circular pattern | Features repeated around an axis | done | P0 | M | engine, doc |
| Pattern buttons in the ribbon | UI for the pattern commands | branch (`solid/mirror-move`) | P0 | S | ui |
| Pattern on a path, pattern of bodies or faces | Further pattern kinds | missing | P2 | M | engine |
| Mirror | Mirror features or bodies across a plane | branch (`solid/mirror-move`) | P0 | M | engine, kernel |
| Move / copy body | Translate or rotate a body, by drag handles or values | branch (`solid/mirror-move`: typed values; drag handles missing) | P0 | M | engine, ui |
| Align | Move a body so one face or edge meets another | missing | P2 | S | engine |
| Sweep | Profile along a path, with guide rail and twist options | missing | P1 | L | kernel (K12, K10) |
| Loft | Solid through several profiles, with rails and end conditions | missing | P1 | L | kernel (K12, K10) |
| Rib and web | Thin supports from an open sketch line | missing | P2 | M | kernel |
| Emboss / deboss | Raise or sink sketch text or profiles on a face | missing | P2 | M | kernel |
| Draft | Tilt faces by an angle from a pull direction | missing | P1 | L | kernel (K12) |
| Push/pull a face | Move or offset a face directly; the model follows | missing | P1 | M | kernel, engine |
| Replace face, offset face, delete face | Direct edits of faces | missing | P2 | L | kernel |
| Split body / split face | Cut a body or face with a plane, surface or sketch | missing | P1 | M | kernel |
| Scale body | Uniform and non-uniform scale | missing | P2 | S | kernel |
| Coil / helix | Spring and thread shapes | missing | P2 | L | kernel (K10) |
| Primitives | Box, cylinder, sphere, torus, cone made in one step | missing | P1 | S | engine (kernel already makes them) |
| Physical material and appearance per body | Density for mass; colour and finish for display | missing | P1 | M | doc, render |
| Per-feature errors and warnings | A failed feature says why; the rest still rebuilds | done | P0 | S | doc |

## 5. Surface modelling

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Surface bodies (open shells) | Bodies that are not closed solids | missing | P1 | M | kernel |
| Surface extrude, revolve, sweep, loft | Open curves swept into surfaces | missing | P1 | M | kernel |
| Patch | Fill a closed boundary with a surface | missing | P1 | L | kernel (K10) |
| Offset surface | Surface at a distance from faces | missing | P1 | L | kernel |
| Thicken | Give a surface a thickness to make a solid | missing | P1 | L | kernel |
| Trim, untrim, extend surface | Edit surface boundaries | missing | P2 | L | kernel |
| Stitch / unstitch | Sew surfaces into a solid, or take a solid apart | missing | P1 | M | kernel |
| Ruled surface | Surface between an edge and a direction | missing | P2 | M | kernel |
| Boundary fill | Make solids from the cells that surfaces enclose | missing | P2 | L | kernel |
| Reverse normal | Flip a surface's side | missing | P2 | S | kernel |
| Free-form (subdivision) modelling | Push-and-pull a control cage into smooth organic shapes, then convert to B-rep | missing | P2 | L | new crate |
| Mesh bodies and editing | Repair, reduce, smooth, section STL meshes; convert mesh to B-rep | missing | P2 | L | geom, kernel |

## 6. Sheet metal

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Sheet metal rules | Thickness, bend radius, K-factor, relief shapes per design | missing | P2 | M | doc |
| Flange | Sheet from a profile, and bent walls from edges | missing | P2 | L | kernel |
| Bend | Bend a flat sheet along a sketch line | missing | P2 | L | kernel |
| Flat pattern | Unfold to a flat part; export it as DXF | missing | P2 | L | kernel, io |
| Convert to sheet metal | Turn a shelled body into sheet metal | missing | P2 | L | kernel |
| Hem, rip, corner reliefs | Further sheet details | missing | P2 | M | kernel |

## 7. Construction geometry

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Origin planes and axes shown | XY/XZ/YZ planes and X/Y/Z axes visible and pickable in the viewport | missing | P0 | S | render, ui |
| Offset plane | Plane parallel to a face or plane at a distance | missing | P0 | S | doc, engine |
| Plane at angle | Plane through an edge or axis, rotated by an angle | missing | P0 | S | doc, engine |
| Midplane | Plane halfway between two faces | missing | P0 | S | doc, engine |
| Plane through three points / two edges / tangent to a face | Further plane constructions | missing | P1 | M | doc, engine, kernel |
| Plane along a path | Plane normal to a curve at a distance | missing | P1 | M | doc, kernel |
| Axis through a cylinder, two points, an edge, two planes | Axes for revolve, patterns and mirrors | missing | P0 | M | doc, engine |
| Construction point | Point at a vertex, centre, edge-plane crossing, three planes | missing | P1 | S | doc, engine |
| Construction geometry follows edits | Planes and axes rebuild from their references | missing | P0 | M | doc (needs K8) |

## 8. Assemblies and joints

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Components | Bodies grouped in named parts with their own origin, nested; linked copies (`component.*`) | done | P1 | L | doc, engine, ui |
| Activate a component | Edit one component in place; others shown faded (new features go into the active one: done; fading: missing) | missing | P1 | M | doc, ui |
| External references | Insert another design as a linked component that updates | missing | P2 | L | doc, io |
| Joints | Connect components at joint origins: rigid, revolute, slider, cylindrical, planar done (`joint.*`, solved as an exact tree, loops refused); pin-slot, ball and loops missing | done | P1 | L | doc, engine |
| As-built joints | Joint components where they already sit | missing | P1 | M | doc |
| Joint limits and motion | Ranges and driving a joint (`joint.set_value`): done; animation: missing | missing | P2 | M | doc, ui |
| Rigid groups | Lock several components together | missing | P2 | S | doc |
| Ground / fix a component | Keep a component in place (`component.ground`) | done | P1 | S | doc |
| Motion links and contact sets | One joint drives another; parts stop on contact | missing | P2 | L | doc, kernel |
| Interference check | Find overlapping volumes between components | missing | P1 | M | kernel (K7), engine |
| Bill of materials | List of components with counts and properties | missing | P2 | M | doc, io |
| Exploded views | Move components apart for presentation | missing | P2 | M | doc, ui |

## 9. Inspect

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Point pick with normal | Picking a point on a body returns its position and normal | done | P0 | S | engine, gpu |
| Measure distance and angle | Between any two picked points, edges, faces, bodies (minimum distance) | done (`inspect.measure`, `inspect.measure_at`, INSPECT > Measure) | P0 | M | kernel, engine, ui |
| Measure length, area, radius | Of a picked edge or face | done | P0 | S | kernel, engine |
| Mass properties | Volume, area, mass, centre of mass, moments of inertia | done (`inspect.mass`, INSPECT > Mass Properties) | P0 | M | kernel, engine |
| Section analysis | Cut the view with a plane and show the cut face | missing | P1 | M | gpu, render, kernel |
| Interference | See assemblies | missing | P1 | M | kernel |
| Curvature and zebra analysis | Show surface quality (comb, zebra stripes, curvature map) | missing | P2 | M | gpu, kernel |
| Draft analysis | Colour faces by draft angle | missing | P2 | M | gpu |
| Accessibility (undercut) analysis | For machining and moulding | missing | P2 | M | gpu |
| Body validity check | Report invalid topology or geometry to the user | missing | P1 | S | kernel (validate exists), ui |

## 10. Drawings (2D sheets) and AutoCAD-style 2D drafting

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Drawing sheets | Sheets of chosen size with a title block template we design ourselves | missing | P1 | L | new `drawing` crate, ui |
| Base and projected views | Orthographic views from the model, with hidden lines | missing | P1 | L | kernel (exact projection), drawing |
| Section, detail, auxiliary and break views | Further view kinds | missing | P2 | L | drawing, kernel |
| Views update with the model | Associative views | missing | P1 | M | doc, drawing |
| Dimensions on drawings | Linear, aligned, angular, radial, diameter, ordinate, baseline, chain | missing | P1 | L | drawing |
| Centre marks and centre lines | Automatic marks on holes and axes | missing | P1 | S | drawing |
| Annotations | Notes, leaders, balloons, surface texture, weld and datum symbols, GD&T frames | missing | P2 | L | drawing |
| Tables | Parts list, hole table, revision table | missing | P2 | M | drawing |
| Hatching | Fill closed regions with a pattern (solid, lines at an angle, cross-hatch), associative to the boundary | missing | P1 | M | drawing, geom |
| Layers | Named layers with colour, line type, line weight, on/off, lock, freeze | missing | P1 | M | drawing (and sketch later) |
| Line types and line weights | Dashed, centre, phantom lines; plotted widths | missing | P1 | S | drawing |
| Blocks | Reusable named groups of 2D geometry, inserted many times, editable once | missing | P2 | L | drawing |
| Text and dimension styles | Named styles for font, height, arrows, precision, tolerance display | missing | P2 | M | drawing |
| Grips | Small handles on selected 2D objects to stretch, move, rotate, scale, mirror by dragging | missing | P1 | M | ui (applies to sketches too) |
| Command line with aliases | Type a command or a short alias (`L` for line, `C` for circle); options as prompts | missing | P1 | M | ui (on top of the command registry) |
| 2D modify tools | Move, copy, rotate, scale, stretch, array, mirror, offset, fillet, chamfer, join, break, explode | branch (`sketch/tools` has offset, mirror, fillet, chamfer, patterns, split) / missing for the rest | P1 | M | sketch, drawing |
| Paper space / model space and viewports | Layouts with scaled windows into the model | missing | P2 | L | drawing |
| Print and export to PDF, DXF, DWG | Output drawings (DWG through a clean-room reader/writer of the public spec only) | missing | P1 | L | io |

## 11. Import and export

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| `.chamfera` design file | Our own versioned format, open and save | done | P0 | M | doc, io |
| STEP export | AP214 B-rep for planar and analytic bodies | done | P0 | M | io |
| STEP import | Solids on analytic surfaces | done | P0 | M | io |
| STEP import of B-splines, assemblies with placements, faceted B-reps | Much third-party STEP uses them | missing | P0 | L | io, kernel (K10) |
| STL export | Tessellated bodies | done | P0 | S | io |
| STL import | As mesh bodies | done | P0 | S | io |
| 3MF export and import | Mesh with units and colour, for 3D printing | missing | P1 | M | io |
| OBJ export | Mesh for rendering tools | missing | P2 | S | io |
| DXF export of sketches and faces | 2D profiles for laser and waterjet | missing | P1 | M | io |
| DXF and SVG import into sketches | See sketch | missing | P1 | M | io |
| IGES | Older exchange format | missing | P2 | L | io |
| Canvas image | Place a reference picture on a plane and calibrate its scale | missing | P1 | M | doc, gpu, ui |
| Screenshot / image export | PNG of the viewport | done | P0 | S | gpu, apps |
| Browser open/save/export | Files in the browser build, stored locally | branch (`web/browser-app`) | P0 | M | engine, ui, apps |

## 12. UI and navigation

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Orbit, pan, zoom to cursor, fit, home | Camera navigation through `view.*` | done | P0 | M | render, ui |
| Standard views, look at a face | Front, top, iso; normal to a face | done | P0 | S | render, engine |
| Perspective / orthographic | Projection switch | done | P0 | S | render |
| View cube | Clickable cube with faces and a corner for iso, plus home and fit | done | P0 | M | ui |
| View cube edges and corners, drag to orbit | Click edges and every corner; drag the cube | missing | P1 | S | ui |
| Navigation presets | Mouse mappings matching other CAD tools | missing | P1 | S | ui |
| Visual styles | Shaded, shaded with edges, wireframe, hidden lines | missing | P1 | M | gpu, render |
| Ribbon with workspaces and tabs | Tool groups by task | done | P0 | M | ui |
| Browser (model tree) | Bodies, sketches, parameters; show/hide, context menu | done | P0 | M | ui |
| Browser: origin, construction, components folders | Tree items for planes, axes and components | missing | P0 | S | ui |
| Timeline | Features in order; edit, suppress, delete, rename (F2), show error and broken reference from the context menu | done | P0 | M | ui, doc |
| Rollback marker | Drag to roll back and insert features earlier | branch (`timeline/rollback`) | P0 | M | doc, ui |
| Reorder features | Drag features in the timeline | branch (`timeline/rollback`) | P1 | M | doc |
| Timeline groups | Collapse several features into a group | branch (`timeline/rollback`) | P2 | S | doc, ui |
| Command search | A search box (on a key) that finds any command by name and runs it | missing | P0 | S | ui |
| Keyboard shortcuts | Single keys for line, rectangle, circle, extrude, fillet, measure, dimension, trim, offset, and so on; editable | missing | P0 | S | ui (now only file, undo, fit, home) |
| Radial context menu | Right-click menu laid out around the cursor with the common commands | missing | P1 | M | ui |
| Context menu in the viewport | Right-click on a body, face or sketch for its commands | done (body, face, edge, empty space, sketch) | P0 | S | ui |
| Selection filters | Limit picks to bodies, faces, edges, vertices, sketches, components | partial (bodies, faces, edges; sketch points, curves, construction; no solid vertices or components yet) | P0 | S | engine, ui |
| Box selection | Window and crossing selection by dragging | missing | P0 | M | engine, gpu, ui |
| Select through / select other | Cycle through overlapping items under the cursor | missing | P1 | S | engine, ui |
| Pre-selection highlight | Highlight what a click would pick | missing | P0 | S | gpu, ui |
| Feature dialogs with live preview | A panel per feature with fields and a preview before OK | missing | P0 | M | ui, engine (partial: Extrude shows a translucent preview through `solid.extrude_preview`) |
| Drag manipulators | Arrows and rings on the model to set distances and angles | missing | P1 | M | ui, gpu |
| Undo and redo | Every change undoable | done | P0 | S | engine |
| Themes (light, dark, system) | From theme tokens | done | P0 | S | render, ui |
| Preferences | Units, grid, snaps, navigation, theme in one dialog | missing | P1 | M | ui |
| Autosave and recovery | Recover work after a crash | missing | P1 | M | apps, io |
| Recent files | Reopen designs quickly | missing | P1 | S | ui, apps |
| Browser build | The app in a web page (WebGPU or WebGL 2) | branch (`web/browser-app`) | P1 | M | apps, gpu |
| Scripting and add-ins | Run command scripts; later a plug-in API | done (command scripts via CLI and MCP) | P1 | M | engine, mcp |

## 13. Parameters

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| User parameters with expressions and units | Named values, formulas between them, mm/in/deg | done | P0 | M | doc |
| Parameters window | Create, edit, remove | done | P0 | S | ui |
| Model parameters listed | Every dimension and feature value, renamable, shown alongside user parameters | missing | P0 | S | doc, ui |
| Parameter comments and favourites | Notes and a short list in the browser | missing | P2 | S | doc, ui |
| Functions in expressions | sin, cos, sqrt, min, max, round and so on | missing | P1 | S | doc |
| Design configurations | Rows of parameter values that make variants of one design | missing | P2 | L | doc |
| Import/export parameters | CSV of parameters | missing | P2 | S | io |

## 14. Rendering

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Shaded viewport with edges | Real-time display | done | P0 | M | gpu |
| Appearance library | Colours and finishes (metal, plastic, glass) per body or face | missing | P1 | M | doc, gpu |
| Environment lighting, ground shadows, reflections | Nicer real-time display | missing | P2 | M | gpu |
| Decals | Pictures placed on faces | missing | P2 | M | gpu |
| Offline path-traced render | Photo-quality pictures | missing | P2 | L | new crate |
| Turntable / animation export | Rotating video of the model | missing | P2 | M | gpu, apps |

## 15. CAM (Manufacture workspace, milestone M10)

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Setup: stock, work coordinate system, machine | Define the raw block, part zero and the machine | missing | P2 | M | cam, doc |
| Tool library | Mills, drills, chamfer and ball tools with feeds and speeds | missing | P2 | M | cam |
| Facing | Flatten the stock top | missing | P2 | S | cam |
| 2D contour | Follow a profile, with lead-in/out and tabs | missing | P2 | M | cam |
| 2D pocket | Clear a closed region | missing | P2 | M | cam |
| Drilling | Drill, peck, tap cycles at hole positions | missing | P2 | M | cam |
| 2D adaptive clearing | Constant tool engagement roughing | missing | P2 | L | cam |
| 3D roughing and finishing | Adaptive, parallel, scallop, contour, pencil passes | missing | P2 | L | cam |
| Multi-axis | 3+2 and simultaneous 4 and 5 axis | missing | P2 | L | cam |
| Turning | Lathe operations | missing | P2 | L | cam |
| Simulation | Show the tool removing stock; collision and gouge checks | missing | P2 | L | cam, gpu |
| Post processors as data | G-code output for Grbl first, then LinuxCNC and others | missing | P2 | M | cam, io |
| Additive (3D-print) setup | Orient, support and slice | missing | P2 | L | cam |

## 16. Collaboration and data

| Feature | Description | Status | Pri | Size | Layer |
|---|---|---|---|---|---|
| Design history | Every save as a version; open or restore an older one | missing | P1 | M | io, doc |
| Version comparison | Show what changed between two versions | missing | P2 | M | doc, gpu |
| Projects and folders | Organise designs | missing | P2 | M | apps |
| Shared links and viewing in a browser | Send a read-only view | missing | P2 | L | apps (browser build) |
| Comments and markup on the model | Notes pinned to the model | missing | P2 | M | doc, ui |
| Live co-editing | Several people in one design | missing | P2 | L | doc, engine |
| Git-friendly design files | Text format that diffs and merges | done (JSON `.chamfera`) | P1 | S | doc, io |
| Agent access over MCP | Build, measure and screenshot a design from an agent | done | P0 | M | mcp, engine |
