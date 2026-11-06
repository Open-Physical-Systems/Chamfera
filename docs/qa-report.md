# QA report: end-to-end use of dev (2026-11-06)

About 25 features landed on `dev` in a few hours, each tested on its own. This pass used
them together the way a user (or an agent over MCP) would: eight realistic parts, each
taken through its whole life, the real app driven to screenshots, and hostile input on
every dialog-backed command.

## What was tested

Eight parts, each a script under `examples/qa/` (`chamfera-cli run examples/qa/NN-*.json`)
and each checked by a test in `crates/engine/src/qa_tests.rs` against values derived by
hand (the derivations are in the test comments):

| # | Part | Script | Features used together | Hand check |
|---|---|---|---|---|
| 1 | Parametric enclosure | `01-enclosure.json` | parameters, a constrained sketch driven by expressions, extrude, fillet, shell, offset construction plane, sketch on a construction plane, sketch on a face, counterbored holes from sketch points, rectangular pattern; then width 80 -> 100 | base and lid volume at both widths; 26352 with the fillet suppressed |
| 2 | Flanged pipe | `02-flanged-pipe.json` | extrude, two lofts (join and cut), sweep of an annulus along line-arc-line (join), hole, circular pattern, rim chamfer | 11000π + 2208.33π + 56.25π(100 + 15π) - 588π - 119π/3 = 65 522.27 mm³ |
| 3 | Sheet metal bracket | `03-sheet-bracket.json` | rule, base flange, three flanges (one with offsets and reliefs), flat pattern, DXF export, DXF re-import | volume, flat length, width and area; the DXF reads back as one profile of that area |
| 4 | Hinge | `04-hinge.json` | components, ground, tint, revolute joint driven 0°, 45°, 90°, measure | distances 4, 2.5475, √0.5 |
| 5 | Tapered profiles, mirrored | `05-spline-ellipse-taper.json` | ellipse, closed fit spline, polygon, tapered extrude, mirror of both bodies | tapered hexagon volume; ellipse area 120π; mirrors keep volume |
| 6 | CAM on part 1's base | `06-cam-enclosure-base.json` | open a saved design, remove a body, holes on an inner face, setup, three tools, facing, pocket, drilling, GRBL and LinuxCNC posts | hole positions, drill depth Z-32, M6 / M0 per dialect |
| 7 | Drawing of part 1 | `07-enclosure-drawing.json` | sheet, front/top/right/iso views, horizontal, vertical, radius and diameter dimensions, PDF/SVG/DXF export | 80, 38, 50, 50, R6, Ø6.5 |
| 8 | DXF -> solid -> exchange | `08-dxf-combine-export.json` | DXF import into a sketch, extrude, a second body, Combine cut, STEP/3MF/GLB export, STEP and 3MF re-import | 32000 - 1500π, the STEP body exact |

For every part: undo everything back to an empty design, redo it all, roll the timeline to
the middle and forward again, suppress and unsuppress a feature, save and reopen (volumes
identical to 1e-9), and a timeline reorder where legal (part 1: the offset plane to the
front; part 3: the front flange before the left one, which legitimately changes the volume
by 4 (3π + 14) = 93.70 because the front edge is longer; part 5: refused with a clear
message because the mirror would lose its body).

Hostile input (a QA harness over MCP, `chamfera-cli mcp`): 0, -0, -1, 1e-12, 1e9, ±1e300,
`1e400 mm`, `width / 0`, `nan`, `inf`, `""` on every length, angle and count of extrude,
fillet, chamfer, shell, hole, both patterns, revolve, move, offset face, press pull,
thicken, construction planes, sheet rules, parameters, joints, drawings, CAM tools and
operations, view zoom and grid, units and feature.edit; deleting features, sketches and
parameters that others use; editing a feature whose input became too small; 200 parameter
edits then 200 undos, 200 redos and 200 interleaved undo/redo.

No command panicked or hung, every refusal named the value and the allowed range, and every
failed command left the design unchanged.

## Bugs fixed on this branch

1. **Mirroring a body with spline faces failed** (`solid.mirror` on any extruded spline
   profile, in any plane: "the moved body: a vertex or edge of FaceId#2 is farther than
   0.0000001 mm from its surface"). An extrusion surface keeps its curve in its frame's
   plane coordinates, but the mirror gave it the surface frame `(Rx, -Ry, Rz)`, which
   reflects the curve. It now gets `(Rx, Ry, -Rz)` like a curve, which keeps the curve and
   the outward normal (`crates/kernel/src/topology/transform.rs`). Test:
   `spline_bodies_mirror_with_their_extruded_faces_outward`.
2. **`feature.edit` accepted keys it does not know, and did nothing.** `{"feature": 10,
   "count": 3}` on a pattern replied with the unchanged feature and pushed no undo step, so
   the user's next undo silently removed the pattern itself. Unknown keys are now refused
   by name, and patterns can be edited: `count` (both kinds) and `spacing` (rectangular).
   Test: `feature_edit_changes_a_patterns_count_and_spacing_and_refuses_unknown_keys`.
3. **No Insert STEP in the interface.** `body.import_step` had no button: it is now in
   SOLID > INSERT and the File menu ("Insert STEP…"), reporting how many solids came in and
   how many were left out (the browser build says it needs the desktop app). Test:
   `insert_step_is_in_the_solid_insert_group_and_reports_what_came_in`.
4. **The status bar's hint ran over the body count** below about 1100 px (at 1000 px the
   count was pushed off the window entirely). The right side is now laid out first; the
   hint is cut short with an ellipsis, and with a message the hint gets at most half the
   room. Test: `a_long_status_message_is_cut_short_instead_of_running_under_the_body_count`.

5. **`--theme dark` was lost when a script opened a design** (screenshots of parts 6 and 7
   came out light). Opening a design adds it to the recent list, which re-applied the
   preferences, theme included. A theme forced at start now holds until the user picks a
   theme in the preferences. Test:
   `a_theme_forced_at_start_survives_opening_a_design_until_the_user_picks_one`.

## Open bugs (with repro)

Each open one has an `#[ignore]`d test that fails today, so whoever fixes it has the check
ready; fixed ones say so and name their tests.

1. **A sweep bend near a flat face cannot be joined** (kernel, booleans).
   Repro: `examples/qa/02-flanged-pipe.json` with the straight run 40 instead of 50
   (`qa_a_sweep_bend_near_a_flat_face_joins`): "join: not supported yet: the boolean needs
   the intersection of face `5:end` ... and face `11:segment 1 side 0` ..., and a plane
   neither perpendicular to a torus's axis nor through it cuts a quartic". The two faces
   are 40 mm apart; `face_box` in `crates/kernel/src/boolean/split.rs` bounds a torus face
   by the whole torus (centre ± (R + r) in every direction), so the plane z = 28 seems to
   meet a bend that never goes below z 55.5. A tighter box for a torus patch (its spine
   arc's box grown by the minor radius, from the face's angular range about the axis)
   would let this and most real elbows join. Workaround: keep straight runs longer than
   R + r before a bend.
   **Status: fixed** (`fix/qa-bugs`). Sphere and torus faces are now bounded by the
   parameter ranges they cover (`crates/kernel/src/boolean/patch_box.rs`): the `u` and `v`
   ranges of the loop images, the gaps between them classified by one point each, and the
   exact box of the image of each `(u, v)` rectangle. The bend's box now starts at
   z 68 - 12.5 and the 40 mm run joins with its hand volume. Tests:
   `qa_a_sweep_bend_near_a_flat_face_joins`,
   `a_quarter_torus_face_is_bounded_by_its_quarter_not_the_whole_torus`,
   `a_half_sphere_face_is_bounded_by_its_half`. (The test's own text replacement also
   rewrote the arc's end point before the arc, so the path did not join; fixed too.)
2. **Drawing dimensions do not follow the model** (drawing). Repro: run part 1, then
   `07-enclosure-drawing.json`, then `param.set width 100 mm`: the views redraw at 100 mm
   but the width dimension still says 80 (`qa_drawing_dimensions_follow_the_model`).
   Dimensions store the picked points in view coordinates, not what they were attached to,
   so after any model change they show a stale number without a warning. **Fixed in
   drawings version 2** (design format 30): dimensions keep K8 references to the vertices
   and edges they measure and follow the model; one whose reference is gone is drawn
   dangling in the warning colour. The test now runs.
3. **Ellipses cannot become solids.** `solid.extrude` (with or without taper) of an
   ellipse profile: "ellipses in a profile cannot be swept into solids yet". The Ellipse
   tool draws profiles that the solid tools then refuse; either the kernel takes ellipse
   edges (kernel ellipses exist) or the extrude dialog should say so before the user picks
   the profile. Part 5 keeps the ellipse in its sketch (area checked) and extrudes a
   hexagon instead.
   **Status: fixed** (`fix/qa-bugs`). Ellipses and elliptical arcs reach the kernel as
   their exact rational quadratic B-splines (`EllipseArc::to_nurbs`: quarter-turn
   pieces of the unit circle with weights `cos(Δ/2)`, mapped affinely), so their sides are
   extrusion surfaces, which here are elliptic cylinders; a whole ellipse is two halves.
   Surface sweeps take them the same way. Booleans now take spline sides
   (`kernel/spline-booleans`): join, cut and intersect with planar solids, cylinders and
   other extrudes along the same direction, and holes parallel to the extrusion, are
   exact (`boolean/extrusion_tests.rs`); a cone, sphere or torus reaching a spline side,
   or a cylinder across it, is refused naming the faces. Tests:
   `an_ellipse_extrudes_into_an_elliptic_cylinder` (a = 10, b = 5, 10 high: 500π),
   `a_half_ellipse_closed_by_its_axis_extrudes_to_half_the_volume` (100π),
   `the_nurbs_of_an_ellipse_lies_on_it_and_encloses_pi_a_b`.
4. **Spline profiles cannot be tapered.** `taper` on a spline profile: "a taper on the
   outer loop: spline edges cannot be tapered yet". Part 5 extrudes its spline straight.
   **Status: refused on purpose, with the reason** (`fix/qa-bugs`). A tapered side is the
   curve's offset swept at the draft angle, and the offset of a B-spline or an ellipse is
   not a (rational) B-spline in general, so no exact surface the kernel has can hold it;
   scaling the curve instead would be exact (a general cone) but is not a taper (the
   draft angle would vary along the edge). The refusal now says this and what to do
   (extrude straight, or use lines and arcs), for splines and ellipses alike
   (`crates/kernel/src/sweep/taper.rs`). Test:
   `a_tapered_ellipse_or_spline_is_refused_saying_why_and_changes_nothing`.
5. **Loft profiles cannot have holes.** "lofting a profile with holes": a hollow
   transition takes two lofts (outer join, inner cut), as part 2 does.
   **Status: fixed** (`fix/qa-bugs`). Sections with the same number of holes loft into a
   hollow solid: each hole is matched to the nearest hole of the previous section, its
   edges matched as the outer loop's are, and its bands face into the hole (names
   `band i hole j side k`); the interpolated cross-sections are checked as whole
   profiles, holes included. Different hole counts are refused by name. Tests:
   `a_frustum_with_a_coaxial_cylindrical_hole` (1630π/3),
   `square_tubes_with_two_holes_match_their_holes_by_position`,
   `lofting_rings_gives_a_hollow_frustum`.
6. **Deleting a feature leaves the features that name its edges broken, while other
   dependencies are refused** (naming, for the K8 work). `feature.delete 2` on part 1
   deletes the box although Fillet1 and Shell1 name its edges and faces: both then fail
   ("the edge 2:sweep 0 is no longer there"). Deleting the lid extrude is refused because
   a sketch and a hole use it. `feature_users` should count blend edges, shell faces and
   the other name references too (or deletion should offer to delete the dependants).
   **Status: fixed** (`fix/qa-bugs`). `feature_users` now takes every item whose
   timeline dependencies include the feature (fillets, chamfers, shells, face edits,
   flanges, linked projections, ...) and every item holding a K8 reference signature to
   one of its entities; the refusal reads "Extrude1 is used by Fillet1, Shell1, ...;
   delete or change them first". Test:
   `qa_deleting_a_feature_that_a_fillet_and_a_shell_name_is_refused`.
7. **Mesh bodies are not part of the design.** `body.import_3mf` / `import_stl` /
   `import_obj` add bodies that undo does not remove and save does not keep: in part 8 the
   3MF read back is still there after undoing everything and gone after save and reopen,
   with no warning. At least the save should say mesh bodies are not saved.
   **Status: fixed** (`fix/qa-bugs`). Imported meshes are a document item (`meshes`,
   format 32: id, name and triangles, re-validated on open, shared between undo steps
   so undo does not copy them). Importing, renaming and removing one is one undo step,
   and save and open keep them; a mesh keeps its scene body through rebuilds. The sample
   part stays session-only. Test: `an_imported_mesh_is_saved_undone_and_named_apart`.
8. **Imported STEP bodies keep their names, which can clash.** Part 8's STEP read back is
   called `Extrude1`, like the body it was exported from; name-based references
   (`solid.combine {target: "Extrude1"}`, `inspect.mass {body: "Extrude1"}`) are then
   ambiguous. Imported bodies should get a unique name (`Extrude1 (STEP)`).
   **Status: fixed** (`fix/qa-bugs`). A STEP solid or mesh whose name a body of the design
   already has gets the first free number: `Extrude1 (2)`, `cube (2)`. Tests:
   `a_step_body_read_back_gets_a_name_of_its_own`,
   `an_imported_mesh_is_saved_undone_and_named_apart`.
9. **There is no glTF import**, so a GLB export cannot be read back; 3MF and STEP can.
10. **`chamfera-cli` panics on a closed pipe**: `chamfera-cli exec app.commands | head`
    ends with "failed printing to stdout: Broken pipe" from `println!`. Harmless, but it is
    a panic in non-test code; write with `writeln!` and stop quietly on `BrokenPipe`.
    **Status: fixed** (`fix/qa-bugs`): every line goes through one writer that turns
    `BrokenPipe` into a quiet exit with status 0. Test (`apps/chamfera-cli/tests/cli.rs`):
    `a_closed_stdout_pipe_ends_the_cli_quietly`.

## Smaller findings

- `design_summary` (MCP) and `body.list` report each body's **mesh** volume
  (21326.09 for part 1's base against the exact 21329.13); `inspect.mass` and the
  feature replies give the exact one. An agent checking volumes with `design_summary` is
  0.01% off. The summary should give the exact volume (and say so).
- `inspect.mass` of the spline body in part 5 is 9239.194725 against the extrude reply's
  9239.194899 (area × height exactly): the mass properties integrate spline faces less
  precisely (2e-8 relative).
- `solid.combine` replies with `"solids": []`, so its result cannot be read from the reply.
- `solid.edges` lists solids with `"name": null`.
- `cam.generate` reports `"minutes": -0.0` for operations that failed.
- `cam.tool_add` takes a 1e-12 mm tool; generate then refuses it cleanly ("more than 2000
  passes"), but the tool dialog could refuse diameters below 0.01 mm at once.
- Drilling at the bottom of a deep pocket starts its peck cycle at the stock's retract
  plane: part 6's four 2 mm holes under a 29 mm pocket peck through 29 mm of air
  (`G83 ... R2 Q1`, about 30 pecks per hole). R should be just above the hole's top.
- `solid.pattern_linear` is labelled "Rectangular Pattern" but has one direction, and a
  pattern of a pattern is refused, so a 2 x 2 grid needs sketch points (part 1 does this).
- Hole points are literal numbers: they cannot follow a parameter. Part 1 drives them
  through a constrained sketch, which works but is long-winded.
- Changing a parameter that makes a later feature fail (`lid` 0.5 mm under a 1 mm chamfer)
  is accepted and leaves the failing feature marked in the timeline; that is the usual CAD
  behaviour, but the reply of `param.set` does not mention the broken feature.

## Performance (debug build, this laptop)

The kernel rebuilds the whole timeline on every change, so the cost of a step is the cost
of the part:

| Part | Build (script) | One undo / redo | 200 parameter edits |
|---|---|---|---|
| 1 enclosure | 0.9 s | ~0.03 s | 31 s (155 ms each) |
| 2 flanged pipe | 14 s | ~0.6 s | - |
| 3 sheet bracket | 0.2 s | ~0.02 s | - |
| 8 DXF / combine | 0.2 s | ~0.01 s | - |

Part 2 is dominated by the sweep join (2 s per rebuild): after it, every hole, pattern or
chamfer costs 2 s more, and undoing its 26 steps takes 15 s. The pocket in part 6 (3227
moves) generates in under a second. Incremental rebuilds (rebuild from the changed feature
on, keeping the bodies before it) would make the big parts usable. Done since: see
`docs/performance.md` (undo, redo and edits near the end now take under a millisecond).

## The real UI

Every part was opened in the app with its script (`chamfera --script examples/qa/NN.json
--size 1600x950 --theme light|dark --screenshot ...`, the drawing with `--drawing`) and
looked at: the geometry matches the numbers above (the enclosure's rounded shell and four
counterbores, the pipe's loft, bend and six bolt holes, the bracket and its flat pattern
with the two reliefs, the hinge at 90° with the Measure panel showing 0.71 mm, the tapered
hexagons and mirrored splines, the pocket and drill toolpaths in MANUFACTURE, the four
drawing views with their dimensions, the cut plate). Both themes are sane; nothing in the
ribbon, browser, timeline or panels is clipped or overlapping at 1600 px, and with the fix
above the status bar no longer runs under the body count at 1000 px (the bracket shot
shows the ellipsis). The exported PDF (A3) renders the same sheet (`pdftoppm`).

Kept in `docs/screenshots/`: `qa-enclosure-light.png`, `qa-flanged-pipe-dark.png`,
`qa-sheet-bracket-light.png`, `qa-enclosure-drawing-dark.png`. All of them (both themes,
every part) are written to `target/qa/shots/` by
`chamfera --script examples/qa/NN-*.json --size 1600x950 --theme T --screenshot ...`.

## UX issues worth fixing

- **Duplicate body names in the Browser.** After part 8 the Browser lists three bodies all
  called `Extrude1` (the cut plate, its STEP copy and its 3MF copy); nothing tells them
  apart, and scripts cannot name one of them (open bug 8).
- **Finished sketches stay drawn.** Part 2 has seven sketches; their circles show as
  dashed rings at the loft-sweep seam and across the flange. A Browser eye per sketch (or
  hiding sketches that a feature consumed, as other CAD tools do) would clean this up.
- **The construction plane is drawn as a large orange sheet through the model** (part 1:
  the lid's offset plane crosses the whole box). It could be smaller, fainter, or hidden
  once a sketch on it is finished.
- **The timeline is icons only.** With 14 items (part 2) the user has to hover each to
  know which loft or sketch it is; names under the icons, or at least the feature name for
  the selected one, would help.
- **The Browser panel changes width** with its content (275 px for the enclosure, 247 px
  for the hinge and CAM shots), so the viewport shifts between designs.
- **Edges vanish on coincident bodies.** In part 8 the plate and its two re-imported copies
  sit on top of each other and the visible-edge lines are lost in the z-fight. Placing
  imported copies where the user can see them, or warning that they coincide, would help.
- **Ribbon notes for the ribbon work** (not changed here): Insert STEP is now in SOLID >
  INSERT's dropdown next to the Insert Mesh big button; a user looking for STEP might
  expect it as its own big button or in a File > Insert submenu. "Rectangular Pattern" has
  one direction only (see above), so the label overpromises. The SHEET METAL tab hides
  SOLID's MODIFY group, so filleting a sheet part means switching tabs.
- **The MCP summary gives mesh volumes** (see Smaller findings): agents checking their
  work against hand values get a 0.01% error and may chase a bug that is not there.
- **Hole points cannot take expressions**, and patterns cannot be patterned: a parametric
  bolt grid needs a constrained sketch of points (part 1), which is far from obvious.

## User guide pass (2026-11-07)

Writing `docs/user-guide/` meant doing every step it describes in the real app (screenshots
with `chamfera --script ... --screenshot` and the start-up options) or with the commands, and
checking the numbers by hand. Where a step did not work as a user would expect, the guide
describes what the app does today and the problem is listed here.

### Sketching, solids, construction, getting started
1. Right-click menus grey out Press Pull and Measure with "Press Pull is planned" / "Measure is planned", although both tools exist (`chamfera --script examples/boss-on-plate.json --context-menu 600,300` on a face, `330,350` on an edge; `crates/ui/src/context_menu.rs`). **Fixed** (`fix/guide-bugs`): the face menu's Press Pull opens the tool on that face, and Measure (face, edge and body menus) opens Measure with what was clicked as its first pick.
2. The heads-up length field shows its unit twice before you type ("40.00 mm mm", the value running over the field's name): `--script examples/sketch-snaps.json --sketch-input "tool line; click 0,0; hover 40,1"`. `heads_up` in `sketch_ui.rs` appends a hard-coded " mm" to a value that already has its unit (an inch design likely shows "in mm"). **Fixed** (`fix/guide-bugs`): the live value shows its unit once; a typed plain number shows the design's unit ("25 in" in an inch design).
3. The Revolve and Circular Pattern dialogs cannot use a construction axis (only sketch lines, or X, Y, Z through the origin; Move likewise, and Mirror only the origin planes); `axis_feature` works from a script. **Fixed for Revolve and Circular Pattern** (`fix/guide-bugs`): both list the built construction axes and pass `axis_feature`. Move and Mirror still take only the origin axes and planes (their commands take no construction feature to follow).
4. solid.thicken on a split face's whole name thickens one piece: a 40 x 20 x 10 block's top split at x = 20, `{faces: ["2:top"], thickness: 2}` gives 800 mm³, not 1600, with no warning (fillet and shell take every piece). **Open, needs a naming decision**: Split Face renames the piece in front of the plane (`4:split plane 3 of 2:top`), so `2:top` names only the piece behind it, and a click on that piece in the Thicken dialog also gives `2:top`; widening the whole name would thicken both pieces there too. Pieces that keep the shared name (a cut splitting the top into `2:top#1` and `2:top#2`) are all thickened (`thickening_a_whole_name_thickens_every_piece_of_a_cut_face`); the Split Face case is the ignored test `thickening_the_whole_name_of_a_split_face_thickens_both_pieces`.
5. Two 40 x 40 rectangles overlapping by 20 x 20, before Split All Crossings: sketch.get lists a profile of area 0.0 with one hole, plus one of 1600.
6. Construction planes have no eye toggle in the Browser; only suppressing hides one, which also changes what is built on it.
7. There is no sketch Point tool in the ribbon (sketch.point is script only), and Insert DXF in the interface has no units or scale choice.

### Surfaces, sheet metal, assemblies, inspect, drawings, render
1. surface.trim replies "solids": [] (no trimmed body, no area); status bar ends in an empty list.
2. body.list gives surface bodies a volume (-2091.7 for a trimmed cylinder, 4183.5 for a half-pipe sweep); expected null.
3. Properties window says "Solid (exact)" for a surface body and Volume "could not be measured" (from the code).
4. Edit Feature only edits a surface extrude's distance; surface revolve angle, sweep and patch are view-only (feature.edit {angle} works by script).
5. Trim Surface dialog trims by planes only; trimming by a body or face is script-only.
6. Sheet metal Rules dialog: Thickness "3 in" or "abc" is skipped silently, the old value kept, "Sheet metal rule saved" shown.
7. Sheet metal status messages always use mm, whatever the design's units.
8. Projection (first angle) moves placed views off the sheet without a warning (front 100,100; top 150,200; right 220,30: top goes to y about 0, right to x about -20).
9. Drawing sheets cannot be edited in the app: views, dimensions and annotations cannot be moved or deleted; sheet size and scale cannot be changed (New Sheet is always A3 1:1); only undo goes back.
10. Only the newest sheet is shown; earlier sheets cannot be shown again.
11. The title block date is the UTC date (2026-11-06 for a sheet made at 02:13 local on 2026-11-07).
12. drawing.list reports "hatched_area": -0.0 for views without a section.
13. A planar joint between a plate's top face (centre 30,30) and an r 5 puck's bottom face places the puck at (25,30): the face centre is averaged from vertices, and a circle has only its seam vertex.
14. No UI for joint limits, deleting a joint, renaming or moving a component; a linked copy made from the Browser lands inside the original with no way to move it.
15. The display bar ignores a section set by view.section (shows "XY 0.0 mm"); touching the offset box replaces the cut with an XY one.
16. Section defaults confuse: the default XY cut at 0 leaves only the footprint of a part standing on XY; the XZ normal is -Y, so +25 cuts at Y = -25.
17. Mass Properties takes an unreadable density ("7.85 g/cm3") as 1, with no error. **Fixed** (`fix/guide-bugs`): a density that is not a number above 0 is refused with an error under the field and no result.
18. A linked copy is placed from the original's un-jointed position and reuses its body names.
19. The Explode slider stops at factor 5 although view.explode takes 20; reopening the panel resets it to 2.
20. chamfera-cli run prints no replies when a step fails, only the error (the agent-tools skill says one reply per line).
21. Mass Properties shows inertia as 9.8125e0 with noise like 2.57e-16, and inertia and mass have no units.

### Manufacture, parameters, timeline, history, files, Assistant, preferences
1. Comparing versions panics the GPU renderer, both in the app and in the windowless snapshot: `chamfera --script examples/version-history.json --screenshot x.png` (in an empty folder) and `chamfera-cli snapshot --script ...` both fail validation with "the buffer bound at binding index 0 is bound with size 16 where the shader expects 32". **Fixed** (`fix/guide-bugs`): every body-layout uniform is built from one 32-byte type; the app logs GPU validation errors instead of panicking and the snapshot returns them as errors. The same sweep found and fixed a second panic: a body with an empty mesh sliced an empty vertex buffer.
2. Screenshot runs read the user's real preferences and Assistant key, and could save the preferences: they followed the user's theme, preset and recent list (now also on the welcome panel), and the Assistant page showed the key's last four characters. **Fixed** (`docs/user-guide`): with `--screenshot` the app uses default preferences, never saves them and loads no key.
3. Preferences > Assistant is wider than the window: the body and footer stick out about 100 px past the title bar's close button (`chamfera --preferences assistant`).
4. design.open on a non-design file shows a parser message: "not a Chamfera design: invalid type: integer `999`, expected struct Document at line 1 column 3" (examples/dxf-plate.dxf).
5. The `--preferences` error message lists general, navigation, units or grid, but leaves out assistant, which works.
6. The ribbon's Post button always writes LinuxCNC; GRBL is only in the File menu, with no hint in the ribbon.
7. The Add Tool dialog has no bull nose mill, although the 3D operations take one; only cam.tool_add with a corner radius adds it.
8. Suppressing the only feature in a configuration makes config.activate reply "volume": -0.0.
9. No check that a drill and its hole match: a flat end mill is taken for Drill, and a 5 mm drill in a 6 mm hole gets no warning.
10. The Assistant's recorded demo runs "Extrude thickness" before "Finish Sketch".
