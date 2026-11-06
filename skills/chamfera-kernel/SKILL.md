---
name: chamfera-kernel
description: "Use when working on geometry (crates/geom), the B-rep kernel (crates/kernel) or the sketch constraint solver (crates/sketch): topology, curves, surfaces, intersections, booleans, tessellation, tolerances, clean-room sources."
---

# The kernel and the solver: our own, clean-room

Chamfera writes its own B-rep kernel and constraint solver. This is the hardest part of the project, so it is built in small rungs (`docs/kernel-ladder.md`), each one tested before the next.

## Sources you may use
- Textbooks: Mantyla, *An Introduction to Solid Modeling*; Hoffmann, *Geometric and Solid Modeling*; Piegl and Tiller, *The NURBS Book*; Patrikalakis and Maekawa, *Shape Interrogation for Computer Aided Design and Manufacturing*; Farin, *Curves and Surfaces for CAGD*; Stroud, *Boundary Representation Modelling Techniques*; Ericson, *Real-Time Collision Detection* (for robust predicates).
- Papers and public specifications (ISO 10303 for STEP entities).
- Permissive code for reference only (MIT/Apache/BSD), credited where an idea is adapted.
- Never OpenCASCADE, FreeCAD, SolveSpace, CGAL (GPL parts) or any other copyleft kernel or solver, not even to read.

## Design rules
- Exact geometry decides: which face a point is on, whether solids touch, depths. Meshes only display and export.
- One tolerance policy: `LINEAR_TOLERANCE` and `ANGULAR_TOLERANCE` in `crates/geom/src/lib.rs`. Don't invent local epsilons; if a case needs another, add a named constant with a comment saying why.
- Topology and geometry are separate: topology (body, shell, face, loop, edge, vertex) refers to geometry (surface, curve, point) by id.
- Operations are functions from valid bodies to a new valid body or a `KernelError`. They never mutate their input and never return an invalid solid. Validate topology (closed shells, consistent orientation, Euler-Poincare) in debug builds and tests.
- Every face, edge and vertex an operation creates carries a name derived from what made it (feature id + profile curve + role), so references survive edits (persistent naming, M4). Design for this from the first rung.
- Analytic surfaces (plane, cylinder, cone, sphere, torus) stay analytic; NURBS only where needed.

## Solver rules (sketch)
- Unknowns are point coordinates and curve parameters; constraints are residual equations.
- Newton or Levenberg-Marquardt with a bounded iteration count; report under- and over-constrained (degrees of freedom) instead of failing silently.
- A solve that fails leaves the sketch at its last valid state.
