# Kernel ladder

Chamfera's B-rep kernel is written from scratch. Each rung is finished, with tests against analytic values, before the next starts. Rules: `skills/chamfera-kernel/SKILL.md`.

| Rung | What | Done when | Status |
|---|---|---|---|
| K0 | Geometry basics: points, vectors, frames, transforms, tolerances, robust orientation predicates | Unit tests incl. near-degenerate cases | Done: `Frame` (origin planes XY/XZ/YZ), `Transform` (rigid and affine), exact `orient2d`/`orient3d`/`incircle` after Shewchuk, in `crates/geom` |
| K1 | Curves: line, circle/arc, ellipse; evaluation, derivatives, closest point, curve-curve intersection in a plane | Analytic intersection tests | Done: line, segment, circle, arc, ellipse, elliptical arc in 3D (evaluation, derivatives, length, closest point, box, reverse); planar line/segment/circle/arc intersections with tangent and overlap cases, in `crates/kernel` |
| K2 | Topology: body / shell / face / loop / edge / vertex with half-edges; Euler operators; validity check | Euler-Poincare and orientation checks on hand-built cube, prism, cylinder | Done: half-edge `Body` in typed-id arenas with curves (K1) and planes by id; MVFS/KVFS, MEV/KEV, MEF/KEF, KEMR/MEKR, KFMRH/MFKRH; topology and geometry validity checks; cuboid, prism with holes (genus) and cylinder topology (lateral surface waits for K3); exact planar area and volume; validated serde, in `crates/kernel/src/topology` |
| K3 | Surfaces: plane, cylinder, cone, sphere, torus; point/normal evaluation, closest point, inside-face test | Analytic tests | |
| K4 | Sweeps: extrude and revolve a planar profile (lines + arcs, with holes) into a valid solid | Volume, area, centre of mass of box, plate-with-holes, cylinder, cone, torus match closed forms | |
| K5 | Tessellation: faces to triangles within a chord tolerance; watertight meshes | Mesh volume converges to the exact volume; no cracks | |
| K6 | Surface-surface intersection: plane/plane, plane/cylinder, plane/cone, plane/sphere, cylinder/cylinder (coaxial and orthogonal first) | Analytic intersection curves | |
| K7 | Booleans (union, subtract, intersect) on planar and analytic solids: face splitting, classification, sewing | Oracle parts; tangent, coincident and touching cases | |
| K8 | Persistent naming of every created face, edge and vertex | Names survive a parameter change in the oracle parts | |
| K9 | STEP AP214/AP242 export of B-rep (then import) | Round-trip of oracle parts | |
| K10 | NURBS curves and surfaces (The NURBS Book), and their intersections | Analytic and convergence tests | |
| K11 | Edge blends: constant-radius fillet and chamfer between planar and analytic faces, then rolling-ball blends in general | Fillet volumes against closed forms | |
| K12 | Shell, draft, sweep along a path, loft | Oracle parts | |
