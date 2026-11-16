# Chamfera: project plan

*Updated 2026-11-16, after the v0.1.0 release (2026-11-07). CAD from scratch first, CAM later. SolidCraft is a reference only, not a dependency.*

---

## 0. Name: **Chamfera** (proposed)

From *chamfer*, the bevelled edge every CAD and CNC user knows, made into a coined word.

| Check (2026-11-06) | Chamfera |
|---|---|
| Web search, exact word | no company, product or brand found |
| crates.io / npm / PyPI | all free |
| GitHub user/org `chamfera` | free (1 unrelated repo, "chamferacademy") |
| Domains .com / .io / .dev / .app | no DNS records (likely unregistered) |
| **USPTO** (official search, 2026-11-06) | **0 hits** for CHAMFERA; 0 for CHAMFORA, CHAMFRA*, SHAMFER*. Closest live marks: CHAMPERA (sports-education app, Cl. 9/41, pending, office action), CHARMERA (cameras, Cl. 9), CHAMFERCUT/CHAMFERMATE/CHAMFER SKIVING (machine tools, Cl. 7/42). No live software mark is spelled like ours |
| WIPO Brand DB, TMview, PDKI | not checked: these sites block automated requests; search by hand |

**Rejected: Millform.**
- Millform Australia Pty Ltd, a steel manufacturer, claims "Millform" as its trade mark.
- Millers Oils sells "Millform VO", a metalworking fluid.
- MILFORM is registered at the USPTO (Reg. 2386101, Mills Industries) and is reported in Milacron's machinery portfolio.

Those are manufacturing and machining fields, which is too close.

Other clean backups: **Kerfora** (no web hits; .io/.dev/.app free), **Zerodatum**, **Torusly**, **Cadwright**.

**Limits of this check:** these are web searches, not official registry searches. Before going public:
1. Search the exact name and sound-alikes in Classes 9 and 42 (software) at **WIPO Global Brand Database** (branddb.wipo.int), **USPTO Trademark Search**, **TMview** (EU and national offices) and **PDKI/DJKI** (pdki-indonesia.dgip.go.id) if you are in Indonesia.
2. Register the domain and GitHub org early; they're cheap.
3. To own the name, file in Class 9 (downloadable software) and Class 42 (SaaS) in your home country first, then use Madrid (WIPO) to extend.

---

## 1. Goal and scope

A Fusion-style parametric 3D CAD application written from scratch. You sketch with constraints, build features on a timeline, change a parameter, and the model rebuilds. CAM (a Manufacture workspace) comes after the CAD core is solid.

- **Language:** Rust, for speed, safety and a wasm/browser build.
- **Licence:** MIT OR Apache-2.0.
- **Runs on:** desktop (Linux/Windows/macOS) and browser, plus a headless CLI and an MCP server for agents.
- **Not copied:** no Fusion icons, UI text, help, file formats or post processors. "Fusion-style workflow" only.

## 2. What we take from SolidCraft (as a reference)
We take the ideas and the agent system, not the code. Reading or adapting their code is legal (MIT/Apache, attribute it), but we write our own.

**The agent system (we copy it on day 1):**
- `CLAUDE.md` contains one line, `@AGENTS.md`.
- `AGENTS.md` is a short router: hard rules, each naming the script that enforces it, plus a table "working on X → read skill Y".
- `skills/<name>/SKILL.md` holds task-specific instructions, loaded only when needed.
- `cargo xtask ci` machine-checks licences, layering, wasm build, sorted lists, and that docs name only real files.
- **Everything is a command:** `session.execute(id, json)` is shared by the UI, CLI, scripts and MCP. Agents can build a part, `measure` it and screenshot it, so they prove a result instead of claiming it.
- **Oracle tests:** analytic volumes, areas and centres of mass for reference parts.
- **Crash safety:** no `unwrap`/`panic`/`unsafe` outside tests, input treated as hostile, a "hostile params never panic" test.
- **Pending files:** each change adds one new file instead of editing a shared one, so parallel agents don't conflict.
- **Clean-room rules:** never copy commercial CAD; never read GPL/LGPL code (OpenCASCADE, FreeCAD, SolveSpace, CadQuery, OpenCAMLib).

**Architecture lessons:**
- Strict layers checked by CI.
- The kernel sits behind one crate boundary (they're swapping truck for SMLib now, and that boundary is what makes it possible).
- Exact geometry for model decisions; meshes only for display and export.
- Persistent face/edge names so references survive edits.

---

## 3. Architecture

```
L0  geom      vectors, transforms, curves/surfaces (line, arc, NURBS), tolerances, meshes
L1  sketch    2D entities + constraint solver (OURS) + profile/region detection
L1  kernel    B-rep: topology (solid/shell/face/loop/edge/vertex), primitives,
              extrude/revolve/sweep/loft, booleans, fillet/chamfer, tessellation   ← behind a trait
L1  render    camera math, scene description (no GPU code here)
L2  doc       parameters + expressions + units, feature timeline, incremental recompute,
              persistent naming, design file format (versioned)
L3  io        STEP AP214/AP242, STL, 3MF, OBJ, DXF/SVG in and out
L4  engine    Session + command registry (everything is a command), undo/redo
L5  ui        egui + wgpu desktop/browser front end; mcp server
apps          chamfera (desktop), chamfera-cli (run scripts, export, mcp), chamfera-web
later: cam (L1), drawings, assemblies/joints, render
```

### The B-rep kernel: our own (decided 2026-11-06, Option B)
The kernel is 60–70% of the difficulty in CAD, so we build it in small rungs, each tested against analytic values before the next: see `docs/kernel-ladder.md` (K0 geometry → K4 extrude/revolve → K7 booleans → K11 fillets). Sources are textbooks, papers and public specs only (`skills/chamfera-kernel/SKILL.md`). The app layers above (sketcher, timeline, UI) are built in parallel against the kernel's interface, so the kernel being young doesn't block them.

### Things we write ourselves from day 1
- **The constraint solver:** Newton/Levenberg-Marquardt over the sketch equations, degree-of-freedom analysis (under- or over-constrained shown in colour), dimensions driven by parameters. It is a well-documented problem, it's very visible, and it's our own.
- Parameters, expressions and units.
- The feature timeline with incremental recompute.
- **Persistent naming (the "topological naming problem"):** designed in from the start, never bolted on later. Faces and edges get names from the feature and profile curve that made them.
- The command system, file format, viewport, camera navigation, selection and picking, and the whole UI.

---

## 4. Milestones (CAD first)

Each milestone ends with a demo you can see plus automated oracle tests.

| # | Milestone | Done when |
|---|---|---|
| M0 ✅ | **Foundation (done 2026-11-06):** repo, CLAUDE.md/AGENTS.md, skills, `xtask ci` (fmt, clippy -D warnings, tests, `cargo deny` licences, layers, wasm), command registry skeleton, CLI | CI green; `chamfera-cli commands` lists a command |
| M1 ✅ | **Viewport (done 2026-11-06):** egui + wgpu window; orbit / pan / zoom-to-cursor, fit, home, standard views, perspective/orthographic; ground grid and X/Y/Z axes; shaded mesh bodies with edges; STL import; picking (point + normal); light/dark/system themes; Browser panel; windowless snapshots; AppImage (8 MB). Not yet: origin planes, a view cube, the browser build run in a browser (it compiles), STL import in the browser | A loaded STL can be orbited and picked on the desktop ✅; the web build compiles ✅ |
| M2 ✅ | **Sketcher (done 2026-11-06):** our own constraint solver (Levenberg-Marquardt, own QR), 18 constraint kinds, DOF analysis with conflict/redundancy reports, closed profiles with holes; sketches stored in the design (format 2) on XY/XZ/YZ; `sketch.*` commands; sketch mode UI: line (chains, snaps, closes), rectangle, circle, arc, selection, constraint buttons, dimensions with inline editing, point dragging, constrained geometry drawn darker. Not yet: sketches on faces, splines, trim/extend/offset, splitting curves at crossings, a sparse solver for very large sketches | A fully constrained sketch survives dimension edits ✅; solver tests with analytic answers ✅ |
| M3 ✅ | **First solids (own kernel K0–K7, done in v0.1.0):** topology, analytic surfaces, exact extrude/revolve, tessellation, booleans, STL and STEP export | A plate with holes exports, and its volume matches the analytic value ✅ (`examples/hole-plate.json`) |
| M4 ✅ | **Parametric timeline (done in v0.1.0):** parameters with expressions, units and configurations; Parameters window; undo/redo; versioned `.chamfera` format; feature timeline with rollback marker, groups and reorder rules; per-feature errors, suppress/edit/delete; persistent naming (K8); incremental rebuild under a millisecond | Change a parameter; the downstream features rebuild ✅ |
| M5 ✅ | **Core features (done in v0.1.0):** sketches on faces and construction planes; revolve, sweep, loft; join/cut/intersect; fillet and chamfer (K11, planar and analytic rims); shell (K12); Hole feature with cosmetic threads; linear and circular patterns; mirror and move/copy; construction planes, axes and points; direct edit and Press Pull; body tools; sheet metal; surfaces. Gaps: fillets ending on curved faces, draft, rib (see v0.2) | A reference part set (bracket, enclosure, flange) all match oracle values ✅ (`examples/patterned-flange.json`, `examples/mirrored-bracket.json`, `examples/shelled-enclosure.json`, `docs/qa-report.md`) |
| M6 ✅ | **Agents (done in v0.1.0):** MCP server, `skills/chamfera-modeling`, windowless snapshots and screenshots; the in-app assistant (bring your own key) | Claude builds a bracket over MCP and verifies it with `measure` ✅ |
| M7 🟡 | **Import and polish:** ✅ STEP import of solids on analytic surfaces, ✅ measure and mass properties, ✅ section view, ✅ appearances, ✅ preferences, ✅ autosave/recovery and version history, ✅ 3MF/OBJ/DXF import. Still to do: STEP with B-splines, assemblies and faceted B-reps (v0.2 unit 1) | Opening a third-party STEP and editing on top of it works (only for analytic solids so far) |
| M8 🟡 | **Components and assemblies:** ✅ components (nested, placements, linked copies, active component, tints), ✅ joints (rigid, revolute, slider, cylindrical, planar; limits; exact tree from grounded components), ✅ joint drag, motion study, interference, exploded views, assembly dialogs. Still to do: joint loops, as-built joints, rigid groups, bill of materials, inactive components faded | A hinge opens to a driven angle and its bodies land where the hand calculation says ✅ |
| M9 🟡 | **Drawings:** ✅ sheets and title block, base, projected, section and detail views with exact silhouettes and hidden lines, associative dimensions (linear to chain/ordinate/baseline), centre marks, notes, GD&T, parts list, hole and thread callouts, SVG/DXF/PDF export. Still to do: user layers and line types, tolerances and styles, more tables and view kinds | A drawing of the enclosure follows a width change ✅ (`docs/qa-report.md`, part 7) |
| M10 🟡 | **CAM (Manufacture workspace):** ✅ setups and stock, tool library, facing, 2D contour, 2D pocket, drilling, 2D adaptive, 3D drop-cutter parallel, waterline, adaptive roughing, pencil, scallop (experimental), holder checks, dexel stock simulation, toolpath cache and background generation, G-code for LinuxCNC and GRBL. Still to do: holding tabs, posts as data with more dialects, tapping, multi-axis, turning | see section 6 |

Rough time with a solo developer plus agents and our own kernel: M1–M2 in 1–2 months, M3 (kernel K0–K7) in 3–6 months, M4–M6 in another 4–6. Estimates, not promises; booleans, persistent naming and fillets are where schedules slip.

---

## 4b. Packaging
Rust builds one native binary per platform with almost no runtime dependencies. We package it per platform:

| Platform | Package | Tool |
|---|---|---|
| Linux | **`.AppImage`** (one file: `chmod +x`, run), plus `.deb` (later Flatpak) | `cargo-packager` (MIT/Apache) or `linuxdeploy` + `appimagetool` |
| Windows | `.msi` / setup `.exe` | `cargo-packager` (WiX / NSIS) |
| macOS | `.dmg` (universal: Apple silicon + Intel) | `cargo-packager`; signing and notarisation need an Apple Developer ID |
| Browser | wasm + static files | `trunk` or `wasm-bindgen` |

- **AppImage notes:** build on an old LTS (Ubuntu 22.04) so the binary runs on older glibc versions. Don't bundle GPU drivers: wgpu uses the system's Vulkan/OpenGL. The CLI (`chamfera-cli`) ships inside the same AppImage and is reachable by a flag or symlink.
- **When:** a CI job builds the AppImage from M1 onward, so every milestone produces a downloadable build. Release builds come from tags.

## 5. Repo layout

```
CLAUDE.md                → @AGENTS.md
AGENTS.md                router: hard rules + enforcing check, skill table
skills/
  chamfera-ground-rules/    clean-room, no GPL, no Autodesk assets, the name
  chamfera-commands/        adding a command, layering
  chamfera-crash-safety/    no unwrap/panic, hostile input
  chamfera-tests/           where tests go, oracle values
  chamfera-sketch-solver/   solver conventions, tolerances
  chamfera-kernel/          the kernel trait, exact-geometry rule
  chamfera-ui/              panels, theme tokens, one camera path
  chamfera-modeling/        (bundled into MCP) how an agent builds a part
crates/  geom sketch kernel render doc io engine ui mcp
apps/    chamfera chamfera-cli chamfera-web
xtask/   ci, licences, layers, skills, wasm
tests/oracle/            reference parts with analytic values
deny.toml                allow: MIT, Apache-2.0, BSD-2/3, ISC, Zlib, BSL-1.0, Unicode-3.0
.github/workflows/ci.yml
```

---

## 6. CAM later (kept from the research, for M10)
- Its own `cam` crate at L3 (`chamfera-cam`, beside `io`), depending on `geom`, `kernel` and `doc`; toolpaths recomputed from the model. Offsets are our own (exact lines and arcs, after Held 1991), not a dependency.
- Build on `cavalier_contours` (arc-keeping offsets, true G2/G3), `i_overlay` (regions), `gcode`/`g-code` (read-back).
- Evaluate **raygeo** (MIT) for adaptive clearing.
- **Forbidden:** OpenCAMLib and FreeCAD CAM/Adaptive (LGPL), pycam, BlenderCAM, CAMotics (GPL).
- MVP: setup/stock/WCS, tool library, Face, 2D Contour, 2D Pocket, Drill, height-field simulation, posts as data (Grbl first, then LinuxCNC), then 2D Adaptive.
- Verification: analytic, G-code read-back, stock simulation, real cuts.

## 7. v0.2: usable for real parts

v0.1.0 shipped on 2026-11-07 (`CHANGELOG.md`). v0.2's aim: open real third-party parts and edit them comfortably. Status per feature: `docs/feature-backlog.md`.

**Being built now, in parallel** (one agent per unit; the files listed are the unit's own):

| # | Unit | Main crates (files) | Done when |
|---|---|---|---|
| 1 | STEP import of B-spline surfaces and curves, assemblies with placements, faceted B-reps | io (`step/`), geom (`nurbs/`), kernel (a B-spline surface type, its tessellation and intersections) | A third-party STEP part with B-spline faces, and an assembly of several parts, import with volumes matching values derived by hand |
| 2 | Draft and Rib/Web features | kernel (new `draft`, `rib` modules), engine (new command files), ui (new dialog files; ribbon entries) | Drafted and ribbed reference blocks match analytic volumes |
| 3 | Drag manipulators in the viewport: extrude distance arrow, move/rotate triad, fillet radius handle | ui (a new manipulator module, the Extrude, Move and Fillet dialogs), gpu, render (overlay shapes) | Dragging each handle runs the same command as typing the value; screenshots show them |

**Queued after, in order of value to a user.** Units with no shared crate can run in parallel (at most four builds at once on this laptop); each unit's "after" names what it must wait for.

| # | Unit | Main crates (files) | After |
|---|---|---|---|
| 4 | General fillets on curved faces: blends ending on curved faces, oblique ellipse and spline rims, pipe T-junctions | kernel (`blend/`) | 1 (B-spline surfaces for free-form blends) |
| 5 | Modelling quick wins: primitives, physical material with density stored per body, body validity check, scale body, align | engine (new command files), doc (body material), ui (new dialog files) | none |
| 6 | 2D drafting: user layers with colour, line type and weight; line types; grips; a command line with aliases | drawing, ui (drawing UI files, a new command line file) | none |
| 7 | CAM: holding tabs on 2D contours, posts as data with more dialects, tapping cycles | cam, engine (`cmd/cam.rs`), ui (`cam_ui.rs`) | none |
| 8 | Viewport and selection polish: origin planes drawn in the viewport, view cube edges, corners and drag, select other, inactive components faded | ui (`viewcube.rs`, `lib.rs`), render, gpu | 3 (same files) |
| 9 | Surface modelling, second pass: surface loft, offset surface, untrim, extend, unstitch, reverse normal | kernel (`surfacing/`), engine (`surface.rs`, `cmd/surface.rs`), ui (`surface_ui.rs`) | 1 |
| 10 | Sketch gaps: SVG import, intersection curve, copy and paste, control-point spline tool, polygon by edge, arc slot | io (a new SVG reader), sketch, engine (`cmd/sketch.rs`), ui (sketch files) | none |
| 11 | Assembly gaps: as-built joints, rigid groups, bill of materials, joint loops | doc, engine (`assembly.rs`, `cmd/joint.rs`), ui (`assembly_ui.rs`) | none |
| 12 | Drawing styles and tables: tolerances, dimension and text styles, hole and revision tables, auxiliary and break views | drawing | 6 (same crate) |
| 13 | Coil, modelled threads, emboss | kernel (`sweep/`), engine | 1 |

Recent files are already done (File > Open Recent, the welcome screen). Every unit adds its buttons to the one tool table in `crates/ui/src/ribbon.rs`; those one-line additions are the only expected overlap and merge by hand.

**Release:** v0.2.0 when units 1 to 3 are merged, `cargo xtask ci` passes and the release checklist (`docs/release-checklist.md`) is done.
