# Changelog

All notable changes to Chamfera. Versions follow [Semantic Versioning](https://semver.org);
before 1.0 a minor version may change the design file format or the commands.

## [0.1.0] - unreleased

The first public release: a parametric 3D CAD application with its own sketch solver,
feature timeline and B-rep kernel, written from scratch in Rust. Numbers in brackets are the
pull requests on GitHub.

### Kernel (B-rep, written from textbooks and papers)
- Frames, transforms, exact predicates, curves and planar intersections (K0-K1) [#3].
- B-rep topology for solids with Euler operators and validity checks (K2) [#6].
- Curved surfaces, and exact extrude and revolve of profiles with arcs (K3-K4) [#13].
- Watertight tessellation of solids for display and export (K5) [#8].
- Surface-surface and curve-surface intersection, point-in-solid classification (K6) [#14].
- Booleans: join, cut and intersect [#17]; on extruded spline and ellipse faces too [#63].
- NURBS curves, sketch splines and extruded spline solids (K10) [#39].
- Fillets and chamfers, including box corners and hole rims (K11) [#19].
- Shell: hollow a solid with open faces (K12) [#26].
- Robust topological naming with one resolver and stored signatures (K8) [#51].

### Sketcher
- Sketch model, our own constraint solver, degrees-of-freedom analysis and profiles [#4].
- Sketch mode: draw, constrain and dimension on a plane or on a solid's face [#5, #15].
- Tools: trim, extend, split, fillet, chamfer, offset, mirror, patterns and arcs [#22].
- Shift-straight lines, object snaps and typed values [#30].
- Polygon, slot, ellipse and elliptical arc; project model edges [#36].

### Solid modelling
- Extrude with extents, two sides, start offset, taper and live preview [#11, #35].
- Revolve and exact round holes [#15]; the Hole feature: simple, counterbore, countersink [#33].
- Linear and circular patterns [#18]; Mirror and Move/Copy bodies [#32].
- Fillet and chamfer features with edges named across rebuilds [#23].
- Construction planes, axes and points [#28].
- Sweep and Loft [#40].
- Direct edit: offset, delete and move faces; Press Pull [#45].
- Body tools: Combine, Split Body, Split Face, Thicken, Remove [#49].
- Sheet metal: rules, base flange, flanges with bends, reliefs, flat pattern [#47].
- Surface modelling: open shells, surface extrude, revolve and sweep, patch, stitch, trim [#53].

### Parameters and timeline
- Parameters: expressions with units that drive dimensions [#7]; v2 with configurations [#55].
- Timeline: explicit order, rollback marker, groups and reorder rules [#37].
- Incremental rebuild: undo and late edits in under a millisecond [#60].

### Assemblies
- Components and joints [#38].
- Joint limits and drag, motion study, interference and exploded view [#52].

### Drawings
- Sheets with views, hidden lines and dimensions [#41].
- Associative dimensions, silhouettes, section and detail views, GD&T and a parts list [#56].

### Manufacture (CAM)
- 2D milling toolpaths and G-code [#43].
- 3-axis milling: drop-cutter, parallel, waterline, adaptive roughing and stock simulation [#59].

### Render and display
- Display modes, body appearances and the section view [#42].
- Render workspace: PBR materials, a realistic viewport and a CPU path tracer [#61].

### Inspect
- Measure and mass properties [#29].

### Files and data
- STEP AP214 export [#9] and import [#20].
- 3MF, OBJ and GLB export; OBJ and 3MF import; DXF into and out of sketches [#48].
- Autosave, crash recovery, atomic saves and version history [#62].

### Interface
- Viewport with orbit, pan, zoom, picking and STL import [#2].
- Redesign: original icon set, ribbon, browser tree, timeline bar and view cube [#16].
- Command search and keyboard shortcuts [#24].
- Box selection and multi-select [#25]; right-click menus, selection filter and hover highlight [#34].
- Preferences: navigation presets, design units and grid [#44].
- The ribbon comes from one tool table, with collapsing groups and a workspace switcher [#50].
- Help > About Chamfera shows the version and the third-party licences.

### Agents and the assistant
- MCP server: AI agents such as Claude Code build parts in Chamfera [#10].
- The in-app assistant: Claude designs live in the app (desktop; bring your own API key) [#57].

### Platforms and packages
- Linux AppImage; Windows zip and per-user installer; macOS universal disk image [#12].
- The browser app (WebGPU or WebGL 2), with files through the browser and GitHub Pages [#21].
- None of the packages are signed yet: Windows SmartScreen and macOS Gatekeeper warn on first run.

### Quality
- End-to-end QA parts and fixes [#54, #58]; test housekeeping [#1, #27, #31, #46, #64].

[0.1.0]: https://github.com/Open-Physical-Systems/Chamfera/releases/tag/v0.1.0
