---
name: chamfera-modeling
description: "Use when building a part in Chamfera as an agent through its MCP server (or chamfera-cli scripts): planning the numbers, drawing, constraining and dimensioning a sketch, driving it from parameters, and checking DOF, profile areas and a snapshot."
---

# Building a sketch-based part over MCP

The MCP server (`docs/mcp.md`) has six tools: `commands` (ids and parameter docs), `run` (one command), `batch` (many steps, stops at the first error), `snapshot` (a PNG of the viewport), `design_summary` (parameters, sketches, bodies) and `skill_reference` (this guide). Units are mm and radians; Z is up.

## 1. Plan the numbers first
Write down every dimension, then work out by hand what the finished sketch must report:
- DOF: each point has 2, each circle 1 more (its radius); each constraint removes what it fixes. A finished sketch has DOF 0 and status `FullyConstrained`.
- Each profile's area, net of its holes. Example, an 80 x 50 plate with two 16 mm holes: plate 80 * 50 - 2 * pi * 8^2 = 4000 - 128 pi = 3597.88 mm², each hole 64 pi = 201.06 mm².

## 2. Look up the commands
`commands {"prefix": "sketch."}` lists the sketch commands and their parameters. The main ones:
- `sketch.create {"plane": "xy"}` (or `xz`, `yz`) makes a sketch and makes it active.
- `sketch.rectangle {"x1", "y1", "x2", "y2"}` adds 4 points, 4 lines (bottom, right, top, left) and 4 horizontal/vertical constraints.
- `sketch.line {"x1", "y1", "x2", "y2"}`; pass `start` or `end` with an existing point id to connect lines.
- `sketch.circle {"cx", "cy", "radius"}` adds a centre point, then the circle. `sketch.arc` takes a centre, a start and an end direction.
- `sketch.constrain {"constraint": {"kind": ..., ...}}` adds a constraint or a dimension. Kinds: `coincident` (a, b), `point_on_line` (point, line), `point_on_circle` (point, curve), `horizontal` and `vertical` (line), `parallel`, `perpendicular`, `tangent`, `equal` (a, b), `fix` (point, at {x, y}), `midpoint` (point, line), `symmetric` (a, b, line); dimensions with a `value`: `distance` (a, b), `point_line_distance` (point, line), `length` (line), `radius` and `diameter` (curve), `angle` (a, b, radians).
- `sketch.dimension {"constraint", "value"}` changes a dimension; `sketch.construction {"curve", "construction": true}` turns a curve into construction geometry, which makes no profile.
- `sketch.get` returns points, curves, constraints, `dof`, `status` and `profiles`.

## 3. Ids come from replies
Points, curves and constraints share one id counter per sketch, starting at 1. Read ids from each reply. Inside one `batch` you cannot read replies between steps, so work the ids out from the counter: in a new sketch a rectangle takes points 1-4, lines 5-8 and constraints 9-12; the next item gets 13. A circle takes its centre id, then its curve id. If unsure, run the drawing steps in one batch, read the replies, then constrain in a second batch.

## 4. Drive dimensions from parameters
Store the design's numbers as parameters (`param.set {"name": "width", "value": 80}`) so a change is one edit. If `commands {"prefix": "sketch.dimension"}` lists an `expression` parameter, give dimensions `{"constraint": id, "expression": "width"}` and they follow the parameter. Without it, set each dimension's value from the parameter yourself, and after changing a parameter run `sketch.dimension` again for every dimension that uses it.

## 5. A worked example
The plate from step 1, fully constrained. Run it as `batch {"steps": [...]}`; the comments give the ids each step creates.

```json
[
  { "command": "param.set", "params": { "name": "width", "value": 80 } },
  { "command": "param.set", "params": { "name": "depth", "value": 50 } },
  { "command": "param.set", "params": { "name": "hole", "value": 16 } },
  { "command": "sketch.create", "params": { "plane": "xy" } },
  { "command": "sketch.rectangle", "params": { "x1": 0, "y1": 0, "x2": 80, "y2": 50 } },
  { "command": "sketch.constrain", "params": { "constraint": { "kind": "fix", "point": 1, "at": { "x": 0, "y": 0 } } } },
  { "command": "sketch.constrain", "params": { "constraint": { "kind": "length", "line": 5, "value": 80 } } },
  { "command": "sketch.constrain", "params": { "constraint": { "kind": "length", "line": 6, "value": 50 } } },
  { "command": "sketch.circle", "params": { "cx": 20, "cy": 25, "radius": 8 } },
  { "command": "sketch.circle", "params": { "cx": 60, "cy": 25, "radius": 8 } },
  { "command": "sketch.constrain", "params": { "constraint": { "kind": "diameter", "curve": 17, "value": 16 } } },
  { "command": "sketch.constrain", "params": { "constraint": { "kind": "diameter", "curve": 19, "value": 16 } } },
  { "command": "sketch.constrain", "params": { "constraint": { "kind": "point_line_distance", "point": 16, "line": 8, "value": 20 } } },
  { "command": "sketch.constrain", "params": { "constraint": { "kind": "point_line_distance", "point": 16, "line": 5, "value": 25 } } },
  { "command": "sketch.constrain", "params": { "constraint": { "kind": "point_line_distance", "point": 18, "line": 8, "value": 60 } } },
  { "command": "sketch.constrain", "params": { "constraint": { "kind": "point_line_distance", "point": 18, "line": 5, "value": 25 } } },
  { "command": "sketch.get" }
]
```

Ids: the rectangle is points 1-4, lines 5-8, constraints 9-12; fix, lengths 13-15; the circles are centre 16, curve 17 and centre 18, curve 19; the dimensions 20-25. DOF: 4 points (8) - 4 horizontal/vertical - 2 fix - 2 lengths = 0; each circle 3 - 1 diameter - 2 distances = 0. So `sketch.get` must report `dof` 0, status `FullyConstrained` and profiles 3597.88 (2 holes), 201.06 and 201.06.

## 6. Check, then look
- Compare `dof`, `status` and every profile area from `sketch.get` or `design_summary` with your hand values. DOF above 0: something can still move, add a constraint. `OverConstrained`: remove the redundant constraint (`sketch.remove {"constraint": id}`).
- `snapshot {"view": "top"}` and look at the image: the outline, the holes where you meant them, construction lines in their own colour. Fully determined geometry of the sketch being edited is drawn in the constrained colour.
- `sketch.finish` ends editing; `design.save {"path": "plate.chamfera"}` saves into the scratch folder (or a path inside an `--allow-path` folder).

## 7. Symmetric parts, moves and copies
- Model half, then `solid.mirror {"bodies": ["Extrude1"], "plane": "yz", "operation": "join"}`: the half joined to its mirror is one symmetric body of twice the volume (`examples/mirrored-bracket.json`). `plane` is `xy`, `xz`, `yz`, `{"feature", "face"}` or `{"origin", "normal"}`; `features: [id]` mirrors a feature's own shape instead, with that operation (a mirrored cut drills the twin hole).
- `solid.move {"bodies": [...], "dx", "dy", "dz"}`, or `"angle"` with `"axis"` and `"origin"`, or `"from"` and `"to"`; `"copy": true` keeps the original. Without `bodies`, both act on the selected body. Body names are the `name` in each solid of a reply.

## When something fails
A failing command changes nothing and its error says what was wrong. In a `batch`, the steps before the failing one stay applied, one undo step each: fix the failing step and run the rest, or `edit.undo` what you do not want.
