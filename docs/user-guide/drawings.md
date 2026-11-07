# Drawings

The Drawing workspace turns the model into 2D sheets you can print or send to a
workshop: a paper sheet with a border and title block, orthographic and isometric views
projected from the exact model, section and detail views, dimensions that stay attached
to the model, and annotations (notes, leaders, feature control frames, surface texture
symbols, centre lines, a parts list and balloons). A sheet is exported as PDF, SVG or DXF.

Drawings are saved inside the `.chamfera` design, so there is no separate drawing file to
keep in step with the model: when the model changes, the views and dimensions redraw.

![An A3 sheet at 2:1 with top, front, right and isometric views and their dimensions](images/drawings-sheet.png)

## Open the Drawing workspace

1. Build at least one solid first (see solid-modelling.md). The view tools are disabled
   until there is one, and their tooltip says "build a solid first".
2. Click the workspace switcher at the left of the ribbon (it reads **DESIGN**) and pick
   **Drawing**. The ribbon now has one tab, **DRAWING**, and the viewport becomes the sheet.
3. To go back to the model, pick **Design** in the same switcher. You cannot switch
   workspace while a sketch is being edited: finish the sketch first.

On the sheet, middle-drag or right-drag pans, the wheel zooms and a middle double-click
fits the sheet in the window. The status bar tells you what the current tool expects next,
and **Esc** drops the tool.

## Start a sheet

1. Click **DRAWING > SHEET > New Sheet**. You get an A3 landscape sheet at 1:1, third-angle
   projection, with a title block (part name, scale, units, date, drawn by).
2. To switch between third- and first-angle projection, click **SHEET > Projection**. The
   projection symbol in the title block changes with it, and the projected views move to
   the other side of their base view (see Limits).

If you place a view before making a sheet, a sheet is made for you. The paper size,
orientation, scale and title block fields can only be set from a script for now
(`drawing.new_sheet` and `drawing.edit_sheet`, below); the New Sheet button always makes an
A3 sheet at 1:1.

## Place views

1. Click **VIEWS > Front** (or **Top**, **Right**; **Isometric** is in the VIEWS list under
   the group's caption).
2. Click on the sheet where the centre of the model should go. The view is drawn with
   visible edges as solid lines, hidden edges dashed and the axes of holes as centre lines.
3. Place the other views the same way. The first view you place is the base view; the
   views placed after it are projected from it and line up with it: a top view keeps only
   its distance above or below the base view, a right view only its distance beside it.
   The isometric view is placed exactly where you click.

Each view shows every solid of the design. A view's own scale, or one body only, can be
set from a script (`scale`, `body`).

## Section and detail views

**Section view** (**VIEWS > Section**):

1. Click two points on a plain view to draw the cutting line.
2. Click where the section view goes. The cut faces are hatched and the view is labelled
   `SECTION A-A`; the next one is B-B, and so on.
3. Before the clicks, the tool's options box has **Look the other way** to flip the
   direction of view. By default you look to the right of the line from your first click
   to your second.

**Detail view** (**VIEWS > Detail**, in the VIEWS list):

1. Click the centre of the area to enlarge on a view, then a point on the circle around it.
2. Click where the detail goes. It is drawn at twice the view's scale unless you type a
   scale such as `4`, `4:1` or `1:2` in the options box first; it is labelled
   `DETAIL B (4:1)`.

## Dimensions

1. Pick a dimension tool in **DRAWING > DIMENSIONS**: **Horizontal**, **Vertical** and
   **Diameter** are buttons; **Aligned**, **Radius** and **Angle** are in the group's list.
2. Click what to measure:
   - Horizontal, Vertical and Aligned: two vertices or edges of one view.
   - Diameter: a circle. Radius: a circle or an arc.
   - Angle: two straight edges.
3. The dimension is placed on the side of the view you clicked on, and the status bar
   shows its value (for example `Dimension 60`). The tool stays on for the next one.

The value is measured on the model, not on the paper, so a dimension on a 2:1 view still
reads the model size.

### Associative and dangling dimensions

A dimension remembers which model vertices and edges it measures, by their persistent
names. Change the model (a parameter, an extrude distance, a feature edit) and the
dimension moves with the view and shows the new value.

If what it measured is gone, for example because you suppressed or deleted the fillet whose
edge it was attached to, the dimension stays at its old place with its old value and is
drawn in the warning colour. That tells you to check it. `drawing.list` reports
it as `dangling`, with the reason.

![After the fillet is suppressed, the R8 and 60 dimensions (attached to fillet edges) are drawn in the warning colour; the Ø16 on the hole still follows the model](images/drawings-dangling.png)

In the picture above, the 60 mm width was picked on the corner vertices of the filleted
block. Those vertices belong to the fillet, so the width dimension goes dangling with it,
even though the block is still 60 mm wide. Pick edges of the base feature when a dimension
should survive the fillets being switched off.

## Annotations

All annotation tools are in **DRAWING > ANNOTATE**. While Note, Leader, GD&T or Surface
Texture is in use, a small options box sits at the top right of the sheet; fill it in
first, then click. These tools, and Centre Line, stay on for the next one until you press
**Esc**.

- **Note**: type the text in the options box, then click where the text starts.
  The text is needed: a click with an empty box is refused.
- **Leader**: type the text, click where the arrow points, then where the text goes.
- **GD&T** (Feature Control Frame): pick the characteristic (straightness, flatness,
  circularity, parallelism, perpendicularity or position), type the value, tick
  **Cylindrical (Ø)** for a cylindrical zone, and give datum letters such as `A B` (the
  datum field is shown only for characteristics that take datums; parallelism and
  perpendicularity need at least one). Click the feature (the leader's end), then where the
  frame goes.
- **Surface Texture** (in the ANNOTATE list): type the requirement (default `Ra 1.6`),
  untick **Material removal required** if needed, then click the surface the symbol sits on.
- **Centre Line** (in the ANNOTATE list): click two circles of one view. The line joins
  their centres and follows the model.

Notes, leaders, frames and texture symbols are placed on the paper: they do not move when
the model changes. Centre lines, dimensions and balloons do.

![Section A-A, detail B at 4:1, dimensions, a centre line, three feature control frames, a surface texture symbol, a leader, a note, a parts list and balloons](images/drawings-annotated.png)

## Parts list and balloons

These need components (see assemblies-and-motion.md); the buttons are disabled until the
design has some.

1. Click **TABLES > Parts List**. A table appears on top of the title block with one row
   per component: item number, name, quantity and material. Linked copies of a component
   count in its quantity. The material is the library material given to the component's
   body (see render.md); it is empty when there is none. Clicking again replaces the list.
2. Click **TABLES > Balloons**, then click a view. Each component in the view gets a
   numbered balloon matching its item number in the parts list.

## Export

Click **DRAWING > EXPORT > PDF**, **SVG** or **DXF** (also in the File menu while the
Drawing workspace is open) and pick a file name. The shown sheet is written at its paper
size: the PDF as one vector page, the SVG in millimetres, the DXF with its lines on layers
(`VISIBLE`, `HIDDEN`, `CENTER`, `DIM`, `BORDER` and so on). The export buttons are enabled once the sheet has a
view.

## From a script

Views, dimensions and annotations take positions in sheet millimetres from the
lower-left corner of the paper. `drawing.add_dimension` snaps its picks to the nearest
projected vertex or edge. This builds the sheet in the first picture (after a 60 x 40 x 15
block with a Ø16 hole and 8 mm fillets, as in `examples/drawing-sheet.json`):

```json
[
  { "command": "drawing.new_sheet", "params": { "paper": "a3", "scale": 2, "part_name": "Filleted block", "drawn_by": "Chamfera" } },
  { "command": "drawing.add_view", "params": { "view": "front", "x": 110, "y": 120 }, "as": "front" },
  { "command": "drawing.add_view", "params": { "view": "top", "x": 110, "y": 215 }, "as": "top" },
  { "command": "drawing.add_view", "params": { "view": "right", "x": 230, "y": 120 } },
  { "command": "drawing.add_view", "params": { "view": "iso", "x": 335, "y": 190, "scale": 1.5 } },
  { "command": "drawing.add_dimension", "params": { "view": "$front.view", "kind": "horizontal", "x1": 50, "y1": 105, "x2": 170, "y2": 105, "offset": -12 } },
  { "command": "drawing.add_dimension", "params": { "view": "$top.view", "kind": "diameter", "x1": 121.3, "y1": 226.3, "offset": 14 } },
  { "command": "drawing.export", "params": { "path": "block.pdf" } }
]
```

The dimension replies give `"text": "60"` and `"text": "Ø16"` with `"associative": true`.
Other commands:

- `drawing.new_sheet {paper: a4|a3|a2|ansi_a|ansi_b, orientation, scale, projection: third|first, name, part_name, drawn_by, date}`
  and `drawing.edit_sheet {sheet, ...the same}`.
- `drawing.add_dimension {view, kind: horizontal|vertical|aligned|diameter|radius|angular, x1, y1, x2, y2, offset}`.
- `drawing.add_section {view, x1, y1, x2, y2, x, y, flip, scale, label}` replies with the
  `hatched_area` in model mm²; `drawing.add_detail {view, cx, cy, radius, x, y, scale, label}`.
- `drawing.add_note {x, y, text, height}`, `drawing.add_leader {points: [{x, y}, ...], text}`,
  `drawing.add_gdt {characteristic, tolerance, diameter, datums, x, y, leader_x, leader_y}`,
  `drawing.add_surface_finish {x, y, text, removal}`, `drawing.add_centre_line {view, x1, y1, x2, y2}`,
  `drawing.add_parts_list {x, y}`, `drawing.add_balloons {view}`.
- `drawing.list` gives every sheet, each view's place, bounds and line counts, and each
  dimension's value, `associative` flag and `dangling` reason.
- `drawing.delete {sheet | view | dimension | annotation}` removes one thing (a view takes
  its dimensions, annotations and the views made from it along).
- `drawing.export {path, format: svg|dxf|pdf}`; `drawing.export_bytes {format}` replies
  with the file as base64.

`examples/drawing-annotated.json` uses every annotation. Its section through both holes
of an 80 x 50 x 12 plate with a Ø8 x 20 pin replies `hatched_area` 928: 80 x 12 less two
8 x 12 holes is 768, plus the pin's 8 x 20 = 160.

## Limits

- **No editing on the sheet yet.** In the app a placed view, dimension or annotation
  cannot be selected, moved or deleted, and there is no Sheet Settings window: Undo
  (Ctrl+Z) is the way back, and `drawing.delete` and `drawing.edit_sheet` do the rest from
  a script or the assistant (assistant.md).
- **One sheet on screen.** New Sheet adds a sheet and shows it, but the app has no way to
  go back to an earlier sheet; all sheets are kept and exported by id from a script.
- **Switching to first-angle projection can push views off the paper.** The projected
  views swap sides about the base view; if the base view is near an edge they land outside
  the sheet. Place the base view near the middle, or switch projection before placing views.
- Sections are straight cuts through plain orthographic views (not through a section,
  detail or isometric view); there are no stepped, aligned or broken-out sections.
- Notes, leaders, feature control frames and surface texture symbols are fixed on the
  paper and do not follow the model.
- The browser app leaves the title block date empty (the desktop app fills in today's
  local date).
- Export PDF, SVG and DXF only; the sheet is exported at its paper size, not scaled to fit.

See also: file-formats.md (other export formats), parameters-and-configurations.md
(changing the model that the drawing follows), timeline.md (suppressing features).
