# Chamfera interface design

The look and layout of the desktop and browser app (`crates/ui`). The goal is a calm,
professional engineering tool: the model is the brightest thing on screen, the chrome
around it is quiet, and every control looks like the others. The workflow is familiar to
Fusion users (workspaces, a ribbon of tool groups, a Browser, a timeline, a view cube);
the layout, icons and wording are our own.

Measurements are in egui points (1 point = 1 pixel at 100 % scale). Every colour is a
token in `crates/render/src/theme.rs`; every measurement below is a constant in
`crates/ui/src/widgets.rs`.

## Layout

```text
┌──────────────────────────────────────────────────────────────────────────┐
│ ◆ File View │ save undo redo          Untitled •                         │ app bar 38
├──────────────────────────────────────────────────────────────────────────┤
│ SOLID  SKETCH                                                            │ tabs 26
│ [tool][tool][tool] │ [tool] │ [tool][tool]                               │ tools 56
│      CREATE ▾        MODIFY      INSERT                                  │ captions 18
├────────────┬─────────────────────────────────────────────────────────────┤
│ BROWSER    │                                                   ┌──────┐  │
│ ▾ Untitled │                                                   │ cube │  │
│   ▸ Origin │                 viewport                          └──────┘  │
│   ▾ Sketch │                                                   home fit  │
│   ▾ Bodies │                                           [ tool dialog ]   │
├────────────┴─────────────────────────────────────────────────────────────┤
│ TIMELINE  [▢]─[▢]─[▢]─[▢] ▌                                              │ timeline 42
├──────────────────────────────────────────────────────────────────────────┤
│ ● 3 degrees of freedom │ tool hint │ message            selection · count │ status 26
└──────────────────────────────────────────────────────────────────────────┘
```

- **App bar** (`chrome`): the app icon, the File and View menus (flat until hovered),
  Save, Undo and Redo as icon buttons, and the design's name centred, with an orange dot
  while there are unsaved changes.
- **Ribbon** (`chrome`): workspace tabs (SOLID; SKETCH appears while a sketch is edited and
  is then the active tab), then tool groups. A tool is a 26-point icon over an 11.5-point
  label. A group's caption sits under its tools; a chevron in the caption means it opens a
  list of everything in the group. Groups are split by hairlines. The tools scroll sideways
  when the window is narrower than the ribbon.
  - SOLID: CREATE (Create Sketch with a plane list, Extrude, Revolve), MODIFY (Parameters),
    INSERT (Insert Mesh, Insert Sample Part).
  - SKETCH: SELECT, CREATE (Line, Rectangle, Circle, Arc), CONSTRAINTS (Sketch Dimension and
    the ten geometric constraints as a two-row grid of small icons), MODIFY (Construction,
    Delete), FINISH SKETCH.
- **Browser** (`panel`, 248 wide, resizable 180-420): a tree. The design is the root; the
  sections Origin (folded at first), Sketches and Bodies fold with chevrons and show their
  counts. Rows are 26 tall, indented 14 per level, with a 16-point icon. The active sketch
  has the `selected` fill and an accent bar on its left; a hidden body is muted. Bodies have
  an eye toggle on the right. Double-click a plane to sketch on it, a sketch to edit it.
- **Viewport**: the model. The view cube sits 78 points from the right edge; Home and Fit are
  a small pill under it. An empty design shows one quiet hint card near the top.
- **Timeline** (`chrome`): one 30-point tile per sketch and feature, in build order, joined
  by short hairlines, ending in an orange marker. Suppressed features are faded with a
  slash; a failed feature has a red outline and a red badge, a feature built but not shown
  an amber badge. Hover shows the name, what it does and any error; double-click edits
  (`sketch.edit`, or the Edit Feature dialog for `feature.edit`); right-click offers Edit,
  Suppress or Unsuppress and Delete.
- **Status bar** (`chrome`): while sketching, a coloured dot (green fully constrained,
  amber free, red over-constrained) and the degrees of freedom, then the tool hint;
  otherwise the navigation hint. Messages follow (errors in red with the error icon). The
  right side shows the selection and the body count.
- **Dialogs** (`panel`, 8 radius, hairline, soft shadow): a title row (icon, title, close),
  a padded body with a muted label column 84 wide so fields line up, and a footer on
  `chrome` with the buttons right-aligned: the primary (accent fill) rightmost, secondary
  (outlined) to its left. Tool dialogs (Extrude, Revolve, Edit Feature) open on the
  viewport's right side under the view cube, away from the model in the middle; Parameters
  opens at the viewport's top-left; Unsaved changes is modal, centred over a dimmed window.

## Spacing scale

| Token | Points | Use |
|---|---|---|
| `S1` | 2 | between tools in a group, between icons in the constraint grid |
| `S2` | 4 | small gaps, icon to text in bars |
| `S3` | 8 | item spacing, dialog grid gaps, between groups |
| `S4` | 12 | dialog padding |
| `S5` | 16 | distance of dialogs from the viewport's edges |

Radii: 4 small (rows, tiles, chips), 5 controls (buttons, fields), 6 medium (ribbon tools,
menus), 8 large (dialogs, cards).

## Type scale

egui's bundled proportional font (Ubuntu Light). No bold: hierarchy comes from size and
colour.

| Token | Points | Use |
|---|---|---|
| `T_CAPTION` | 10.5 | group captions, section headers (upper case, muted) |
| `T_SMALL` | 11.5 | tool labels, status bar, notes |
| `T_BODY` | 13 | body text, menus, fields, Browser rows |
| `T_TITLE` | 14.5 | dialog titles |

## Colour

Surfaces, from the outside in: `chrome` (bars), `panel` (Browser, dialogs), `panel_alt`
(tiles, inactive controls), white or `chrome` for text fields. `border` draws every
hairline. States: `hover` under the pointer, `selected` for the active row, tool, tab or
pressed control, and an `accent` ring for keyboard focus. Content: `text`, `text_muted`
(captions, hints, labels), `accent` (focus, primary buttons, the active tab's underline),
`on_accent` (text on accent), `icon` and `icon_accent` (icons), `success`, `warning`,
`error`. The view cube uses `cube_face`, `cube_edge`, the axis colours and `icon_accent`
for its chamfer.

The interaction accent is blue; the brand accent, the chamfer orange of the app icon, is
kept for the "active part" of icons, the unsaved dot, the timeline marker and the view
cube's chamfer, so it reads as Chamfera without competing with selection.

Contrast (checked by the theme tests): text and muted text at least 4.5:1 on `chrome`,
`panel`, `panel_alt`, `hover` and `selected`; error text 4.5:1; `on_accent` 4.5:1 on
`accent`; icons, status colours and the accent at least 3:1 on every surface they sit on.

## Icons

Masters are SVG files in `assets/icons/ui/`, original artwork (see `ATTRIBUTION.md`).

- **Grid**: 24 x 24, `viewBox="0 0 24 24"`, artwork inside a 2-point margin (20 x 20 live
  area); optically centred.
- **Strokes**: 1.75 wide, round caps and joins, no fills except small accent solids (points,
  the chamfer) and 28 % tinted faces.
- **Corners**: outer corners of rounded shapes have radius 1-1.5; sketch geometry (rectangle,
  cube edges) keeps sharp corners, because those are the things being drawn.
- **Colour roles**: exactly two. The base role is written `#2B313B` and the accent role
  `#E8770E` in the masters; these are markers, not the shown colours. The accent marks the
  "active part" of a glyph: what the tool makes or acts on (the end points of a line, the
  arrow of an extrude, the constrained element of a constraint). Some glyphs (close,
  chevrons, eye off) have no accent.
- **Light and dark**: `crates/ui/src/icons.rs` rasterises each role as its own white mask with
  resvg and draws it tinted with the `icon` and `icon_accent` tokens, so both themes (and
  disabled, error and warning states) come from the theme, never from the files.
- **Sharpness**: masks are rasterised at the exact pixel size they are drawn at (size x
  pixels per point) and placed on whole pixels, so icons are crisp at 1x, 2x and any zoom.
  Sizes in use: 16 (Browser, status, menus), 18 (constraint grid, menus), 20 (app bar,
  timeline), 26 (ribbon tools).
- **New icons**: draw on the grid with the two role colours, run the icon tests
  (`cargo test -p chamfera-ui icons`), and look at the contact sheet:
  `CHAMFERA_ICON_SHEET=/tmp/icons.png cargo test -p chamfera-ui icon_sheet -- --ignored`.

## States

- Hover: `hover` fill (flat controls), the hairline darkens on outlined controls.
- Active (current tool, active tab, active sketch): `selected` fill; tools add a faint
  accent outline, tabs an accent underline, rows an accent bar.
- Focus (keyboard): a 1.5-point `accent` ring outside the control.
- Disabled: icon and label in `text_muted` at reduced alpha; the tooltip says what to do
  first ("Draw a closed profile in a sketch first").

## Screenshots

`docs/screenshots/ui-*.png` show the result in both themes at 1400 x 900 and 1000 x 680. They
are taken with the app itself, for example:

```text
chamfera --theme dark --size 1000x680 --script examples/boss-on-plate.json --screenshot shot.png
chamfera --script examples/parametric-plate.json --parameters --screenshot shot.png
chamfera --script plate-sketch.json --extrude --screenshot shot.png   # Extrude dialog
chamfera --zoom 2 ...                                                  # check at 2x
```
