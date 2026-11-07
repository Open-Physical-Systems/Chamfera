# File formats

Chamfera saves designs in its own `.chamfera` format and reads and writes the common exchange formats. This page lists each one, where to find it, and what it refuses. Every file you open is treated as untrusted: a broken or hostile file gives an error message, never a crash.

| Format | In | Out | Where |
|---|---|---|---|
| `.chamfera` design | File > Open… | File > Save, Save As… | whole design |
| STEP (`.step`, `.stp`) | File > Insert STEP… | File > Export STEP… | exact solids |
| STL | File > Insert Mesh (STL, OBJ, 3MF)… | File > Export STL… | triangle meshes |
| OBJ | Insert Mesh | File > Export OBJ… | meshes (+ `.mtl` colours) |
| 3MF | Insert Mesh | File > Export 3MF… | meshes with colours |
| glTF binary (`.glb`) | no | File > Export glTF Binary… | meshes with colours |
| DXF | File > Insert DXF into Sketch… | File > Export Sketch as DXF…, drawing and flat pattern exports | 2D |
| SVG, PDF | no | DRAWING > EXPORT | drawing sheets |
| G-code (`.nc`, `.ngc`, `.gcode`) | no | File > Post G-code (LinuxCNC / GRBL)… | toolpaths |
| CSV | Parameters > Import CSV… | Parameters > Export CSV… | user parameters |
| PNG | no | RENDER workspace | rendered images (see render.md) |

## Chamfera designs (`.chamfera`)

A `.chamfera` file holds the whole design: parameters, configurations, sketches, the timeline, components and joints, imported STEP solids and meshes, drawings, CAM setups, tools and operations, appearances and the design's units. It is written atomically (to a temporary file, then renamed), so a crash during a save never leaves a broken file, and each save keeps a version in the `.history` folder beside it (see version-history-and-recovery.md).

Open one with **File > Open…** (Ctrl+O) or **File > Open Recent**. A file that is not a Chamfera design is refused with "not a Chamfera design: …". Files from newer versions of Chamfera may not open in older ones.

## STEP

**Export:** **File > Export STEP…** writes every solid of the design as exact B-rep geometry (AP214, one closed shell per solid), in millimetres. Mesh bodies are not written; a design with only meshes is refused ("there are no solids to export").

**Import:** **File > Insert STEP…** reads AP203, AP214 and AP242 files. Each B-rep solid becomes an exact body you can measure, fillet, combine and machine. The status bar says "Inserted 1 STEP solid", and "; 2 left out (not supported yet)" for solids it cannot read yet. A body whose name is taken gets a number, such as `Extrude1 (2)`. Imported solids are not parametric: they have no sketches or features behind them.

Round trip checked: a 60 × 40 × 10 mm plate with a 16 mm hole and 5 mm corner fillets (21 774.78 mm³) comes back from STEP with the same volume and area.

## Meshes: STL, OBJ, 3MF, glTF

**Insert:** **File > Insert Mesh (STL, OBJ, 3MF)…** picks the reader by the file's extension. STL may be binary or ASCII; all are read in millimetres; a 3MF file gives one body per build item. A mesh body can be shown, hidden, measured and exported, and it is saved in the design, but it is not a solid: features and booleans do not work on it.

**Export:** each writes a triangle mesh made from the exact model:

- **File > Export STL…** writes a binary STL of all shown solids, in mm, without colours.
- **File > Export 3MF…** writes the selected bodies (or else all shown ones) with their colours, in mm.
- **File > Export OBJ…** writes the selected (or shown) bodies in mm, with their colours in a `.mtl` file of the same name beside it.
- **File > Export glTF Binary…** writes a `.glb` of the selected (or shown) bodies with colours. glTF is in metres with Y up, as its specification says, so viewers show the right size and orientation.

Meshes approximate curved faces: an STL of the plate above has 476 triangles and a volume of 21 776.20 mm³, 0.007 % from the exact 21 774.78. Model decisions in Chamfera always use the exact geometry; meshes are for display and export.

## DXF

**Into a sketch:** while a sketch is being edited, **File > Insert DXF into Sketch…** draws a 2D ASCII DXF file into it: lines, arcs, circles, ellipses, splines, polylines (LWPOLYLINE and POLYLINE, with bulges) and points become sketch geometry, and ends that meet are joined. After you pick the file, the **Insert DXF** dialog asks for the **Units** (**From the file**: its `$INSUNITS`, else mm; or **Millimetres** or **Inches**) and a further **Scale** (default 1); `examples/dxf-plate.dxf` says it is in inches and comes in scaled by 25.4. From a script: `sketch.import_dxf {path, units?: "mm" | "inch", scale?}`. The status bar says how many curves and profiles came in and how many entities were left out (text, for example). The item is greyed when no sketch is active.

**Out of a sketch:** **File > Export Sketch as DXF…** (while a sketch is being edited) writes it as an R12 ASCII DXF in mm.

**Drawings and sheet metal:** **DRAWING > EXPORT > DXF** writes the drawing sheet (see drawings.md); **SHEET METAL > FLAT PATTERN > Export DXF** writes the flat pattern's outline (see sheet-metal.md).

## SVG and PDF

**DRAWING > EXPORT > PDF** and **SVG** (also in the File menu while the DRAWING tab is open, as "Export Drawing as PDF…" and so on) write the sheet with its views, dimensions and annotations, at the sheet size. See drawings.md.

## G-code

**File > Post G-code (LinuxCNC)…** and **File > Post G-code (GRBL)…** write the first setup's toolpaths. Read manufacture.md before running a file on a machine.

## CSV

The Parameters window's **Export CSV…** and **Import CSV…** move user parameters as CSV with the columns `name, expression, unit, comment` (see parameters-and-configurations.md).

## From a script

```json
[
  { "command": "design.save", "params": { "path": "plate.chamfera" } },
  { "command": "design.open", "params": { "path": "plate.chamfera" } },
  { "command": "design.export_step", "params": { "path": "plate.step" } },
  { "command": "body.import_step", "params": { "path": "plate.step" } },
  { "command": "design.export_stl", "params": { "path": "plate.stl" } },
  { "command": "design.export_3mf", "params": { "path": "plate.3mf" } },
  { "command": "design.export_obj", "params": { "path": "plate.obj" } },
  { "command": "design.export_glb", "params": { "path": "plate.glb" } },
  { "command": "body.import_stl", "params": { "path": "part.stl" } },
  { "command": "body.import_obj", "params": { "path": "part.obj" } },
  { "command": "body.import_3mf", "params": { "path": "part.3mf" } },
  { "command": "sketch.create", "params": { "plane": "xy" } },
  { "command": "sketch.import_dxf", "params": { "path": "outline.dxf", "units": "mm", "scale": 1 } },
  { "command": "sketch.export_dxf", "params": { "path": "outline-out.dxf" } },
  { "command": "drawing.export", "params": { "path": "sheet.pdf" } },
  { "command": "sheet.export_flat", "params": { "path": "flat.dxf" } },
  { "command": "cam.post", "params": { "dialect": "linuxcnc", "path": "part.nc" } },
  { "command": "param.export_csv", "params": { "path": "params.csv" } }
]
```

`drawing.export` takes the format from the path's extension (`svg`, `dxf` or `pdf`) or a `format`; any other extension is refused. `config.export_all {dir, format: step|stl}` writes one file per configuration. The `*_bytes` commands (`design.export_bytes {format}`, `body.import_stl_bytes {data}`, ...) do the same with base64 data instead of paths, for the browser build and agents.

## Limits

- No glTF, SVG, PDF or G-code import; no IGES, Parasolid, ACIS or native files of other CAD products.
- STEP import brings closed solids only, made of planes, cylinders, cones, spheres and tori with line, circle and ellipse edges (no assemblies, colours, PMI or open surfaces yet); solids it cannot read, such as ones with free-form B-spline faces, are listed as left out. Lengths are converted from the file's units to mm.
- DXF import is 2D and ASCII only; binary DXF is refused ("save the drawing as ASCII DXF"), and 3D entities, text, dimensions and hatches are left out (the reply counts them by type).
- Mesh bodies cannot become solids, and cannot be written to STEP.
- In the browser build, Open and Insert use the browser's file picker, Save and Export download the file, and Insert STEP is not available yet.

Related: getting-started.md, drawings.md, sheet-metal.md, manufacture.md, render.md.
