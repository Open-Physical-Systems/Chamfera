# Performance: rebuilds and edits

Every change to a design rebuilds its timeline. This page says how to measure that, what
the session caches so an edit does not rebuild what it did not change, and the numbers.

## Measuring
- `cargo run --release -p chamfera-cli -- bench [--runs N] [script.json]...`, from the
  repository root, runs every example (`examples/*.json`, `examples/qa/*.json`) or the
  scripts named, and prints one table row per design: the full rebuild split into kernel
  (building shapes), booleans, naming and tessellation, then undo, redo and an edit to the
  last feature (restoring it after suppressing it) with the rebuild cache **off** (every
  change rebuilds the whole timeline, as Chamfera did before) and **on**.
- `debug.timings {runs, cached}` gives the same split for each timeline item of the open
  design (`kernel_ms`, `booleans_ms`, `naming_ms`, `tessellation_ms`, `total_ms`), the
  mean and fastest rebuild, and the time spent outside the items (placing components, the
  scene, toolpaths). `debug.cache` says what the caches hold and how the last rebuild used
  them.
- Kernel time is the item's time less its booleans and naming. Booleans the kernel runs
  inside an operation (a fillet's or shell's own trims) count as kernel time; the engine's
  joins, cuts, intersections, holes and combines count as booleans. Tessellation is
  counted against the feature that made the solid.

## The caches (`crates/engine/src/perf.rs`)
- **Checkpoints.** After each timeline item a rebuild keeps what it carries to the next
  item (`solids::Progress`: the bodies so far, each feature's own shapes for patterns and
  mirrors, errors, construction geometry, sheet metal state, and the sketches that follow
  a face) under a 128-bit key. The key hashes the key of the item before it, the item's
  serialized definition, whether it is suppressed (configurations decide that), the
  reference signatures it keeps (K8 naming), and the inputs of every parameter any of its
  strings names: a user parameter's value, unit and effective expression, a model
  parameter's expression or value, recursively (bounded). A seed hashes what any item may
  read: the length unit, the sheet metal rules and every item's name.
- A rebuild hashes the items up to the marker, finds the **last** item whose key it has,
  restores that checkpoint (and moves the followed sketches back into the design), and
  builds only the items after it. Because each key includes the one before it, an item
  changed anywhere invalidates everything after it, exactly the items a full rebuild
  would build differently. An edit near the end builds only the end; undo and redo find
  the whole timeline and build nothing.
- **Meshes.** Each solid's mesh is kept by the exact body: a cheap key (entity counts,
  vertex coordinates, tessellation options) finds candidates and the body is compared in
  full, so a match is the same body and the mesh is the one `tessellate` would make.
  Solids an edit did not change are not tessellated again. Imported STEP bodies use it
  too.
- **Bounded.** Both are LRU maps with a byte limit (checkpoints 256 MB, meshes 128 MB,
  at most 4096 entries each), measured with `Body::approx_bytes` (slots, names, spline
  data) and the mesh sizes. An entry larger than the whole limit is not kept.
  `Session::set_rebuild_cache_limits` changes the limits, `set_rebuild_cache(false)`
  switches caching off.
- **Same result.** The cache must never change what is built. `perf_tests.rs` runs every
  example and QA part in two sessions side by side, one with the caches and one without,
  through the script, undo of every step, redo of every step, and random edits with a
  fixed seed (parameter changes, suppressing features, rolling the timeline, units,
  configurations, undo, redo); after every command the bodies must be equal bit for bit
  (so volumes, face counts and face names), and so must the meshes, errors, constructs,
  sheets, the design (followed sketches) and toolpaths. A hostile design (40 plates, a
  60-deep parameter chain) runs with 64 KB limits and then none, staying within them.

## Hot spots fixed
- Tessellation dominates every curved part (the flanged pipe's torus bend, the swept pipe,
  the revolved parts). Delaunay refinement checked every triangle against the surface in
  each of its up to 48 rounds; it now skips a triangle that is unchanged since it was found
  within tolerance (same corners, same interior sides: corners do not move while
  refining), which gives the same bad triangles in the same order. Smoothing remembers
  each triangle's fit until one of its corners moves. Meshes of all examples are
  identical before and after (checked by hashing every scene mesh).
- Booleans already skip bodies whose bounding boxes do not meet the tool, so the joins
  and cuts measured here are real work; naming is under 0.1 ms everywhere.

## Numbers (release build, this laptop, mean of 3 runs, milliseconds)
Before: the session before this work, which rebuilt everything on every change (the "off"
columns measure the same thing with the caches switched off, and the full rebuild column
before the tessellation fixes). After: caches on and the tessellation fixes.

| Design | Full rebuild before / after | Kernel | Booleans | Tess. after | Undo before / after | Redo before / after | Edit last feature before / after |
|---|---|---|---|---|---|---|---|
| QA 1 enclosure | 17.2 / 17.2 | 3.9 | 6.9 | 6.4 | 10.3 / 0.2 (50x) | 17.1 / 0.2 | 17.4 / 0.4 (43x) |
| QA 2 flanged pipe | 454.9 / 371.0 | 2.5 | 23.5 | 344.9 | 448.9 / 0.2 | 455.0 / 0.2 | 455.8 / 0.6 |
| QA 3 sheet bracket | 23.6 / 23.7 | 22.7 | 0.0 | 1.0 | 15.3 / 0.1 | 23.7 / 0.1 | 23.7 / 0.1 |
| QA 5 spline, ellipse, taper | 28.7 / 28.8 | 9.8 | 0.0 | 18.9 | 15.1 / 0.1 | 28.6 / 0.1 | 32.6 / 3.7 |
| QA 6 CAM enclosure base | 24.1 / 24.0 | 4.1 | 15.1 | 4.0 | 17.3 / 1.0 | 25.0 / 1.9 | 25.2 / 2.0 |
| QA 8 DXF, combine, export | 8.6 / 8.6 | 0.2 | 1.3 | 3.5 | 6.9 / 2.5 | 8.6 / 7.2 | 8.6 / 7.2 |
| hole plate | 52.4 / 49.2 | 0.6 | 15.7 | 32.9 | 30.5 / 0.1 | 52.4 / 0.1 | 52.7 / 0.4 |
| patterned flange | 22.5 / 22.8 | 0.3 | 9.4 | 13.0 | 10.3 / 0.0 | 22.5 / 0.0 | 22.7 / 0.2 |
| revolved parts | 697.3 / 475.3 | 0.2 | 0.0 | 475.0 | - | 699.6 / 0.1 | 700.6 / 0.2 |
| swept pipe and loft | 340.6 / 257.3 | 0.7 | 0.0 | 256.6 | 279.6 / 0.1 | 340.4 / 0.1 | 340.0 / 0.2 |
| surface basics | 47.2 / 40.8 | 0.3 | 0.0 | 40.4 | 47.3 / 0.1 | 47.2 / 0.1 | 47.2 / 0.1 |

Running a whole script (every step rebuilds): QA 1 136 -> 50 ms, QA 2 3107 -> 1593 ms,
swept pipe and loft 3689 -> 264 ms. The goal, at least 5x for an edit to the last feature
of QA part 1 and for undo, is met: 43x and 50x.

Where time still goes:
- An edit that changes a curved solid re-tessellates that solid: the flanged pipe's
  combined body takes about 345 ms (one face of the bend has about 49 000 triangles at the
  default tolerances); refinement, smoothing and flip improvement each take about a third.
- The 3D CAM examples (`cam-3d-*`) spent 0.5 to 5 s after every change working the
  toolpaths out again. Toolpaths are now cached and made on a worker in the app: see
  "Toolpaths" below.
- QA 8 regenerates its 2D toolpaths and DXF profile after every change (2.5 to 7 ms).

## Toolpaths (`crates/engine/src/cam_cache.rs`)
- **Keys.** Each operation's toolpath is kept in an LRU (64 MB) under a 128-bit key of
  what it reads: its definition, tool (with its holder) and setup (their JSON); the
  setup's geometry worked out from the model (stock box, model box, work coordinates,
  clearance, bit for bit); and the geometry it machines. Facing reads nothing more. The
  3D operations and drilling every hole read the whole model: each body's exact data is
  hashed. A contour, a pocket or drilling named holes resolve their persistent names
  through K8 naming and hash what those resolve to (which body; the face or edge, exactly,
  with its name, surface, loops, edges and the faces across them, without ids, which a
  rebuild may renumber), and only the rest of the model they can read: for a pocket its
  walls whole and, of every other face, the edges whose boxes meet the floor's box (the
  points it checks for overhangs lie on them); for named holes every face in reach whose
  surface is a cylinder (a coaxial one's box holds the axis). A feature far from the
  machined faces leaves the key as it was, and the toolpath is taken from the cache.
- **Same result.** `perf_tests.rs` runs every example and QA part, undo and redo of every
  step and random edits with a fixed seed in a session with the caches and one without
  (`set_rebuild_cache(false)` turns the toolpath cache off too) and compares every
  toolpath after every step. `the_toolpath_cache_hits_after_a_far_edit_and_misses_after_an_edit_to_the_machined_face`
  checks the hits and misses by hand on the CAM plate: a slot far from the pocket keeps
  facing, the pocket and the contour (3 hits, drilling misses); deepening the pocket
  misses it and drilling (2 and 2); undo hits all 4.
- **Background generation.** The desktop app turns it on (`Session::set_cam_background`):
  a rebuild then takes what the cache has and leaves the rest as a `CamJob` (a snapshot of
  the CAM section and the bodies) for a worker thread, which runs the same pure function
  (`CamJob::run`; `generate_snapshot` for a whole design). Until the results come back the
  old toolpaths show dimmed (`toolpath_stale`) with a "Regenerating toolpaths…" badge; a
  newer rebuild bumps the generation and the 3D operations' loops stop at their next
  check (`chamfera_cam::cancel`). Results go into the cache and replace a shown toolpath
  only if its key is still current. Commands that read toolpaths (`cam.generate`,
  `cam.post`, `cam.simulate`, `cam.list`, adding or editing an operation or a tool) first
  make whatever is stale on the spot, so replies never depend on the worker; scripts,
  tests and agents never turn it on. `background_generation_gives_the_same_toolpaths_as_synchronous_generation`
  checks a cancelled job, the replacing job and `cam.post` against a synchronous session.

Measured with `cargo run --release -p chamfera-cli -- bench --cam` (mean of 3 runs, ms):
an edit to the part (its last feature suppressed, then restored) with every toolpath
made again on the spot as before; with background generation, the time the UI waits
(the rebuild, the keys and the job's snapshot, the cache emptied first so every 3D
toolpath misses) and the worker's time; and undo and redo from the cache.

| Design | Edit, every toolpath made again | Edit, UI waits (background) | Worker | Undo (cached) | Redo (cached) |
|---|---|---|---|---|---|
| cam-3d-finish | 5056.7 | 1.2 | 5036.2 | 1.0 | 1.7 |
| cam-3d-simulate | 5063.6 | 1.5 | 5047.1 | 0.8 | 1.1 |
| cam-3d-pencil | 31.6 | 0.5 | 32.5 | 0.1 | 0.2 |
| cam-plate | 0.8 | 0.6 | 0.2 | 0.3 | 0.6 |
| cam-holder-collision | 0.3 | 0.2 | 0.1 | 0.1 | 0.3 |

The goal, edits to a modelled part with 3D operations under 100 ms of UI-blocking time,
is met: about 1.5 ms where they took 5 s, the toolpaths arriving from the worker about
5 s later (dimmed until then); undo and redo back to a design generated before take
about a millisecond.

## Background rebuild (design, not built yet)
A rebuild on a worker thread would keep typing in a dialog responsive while a curved
solid re-tessellates. Commands must stay deterministic and sequential, so the design is:
- `Session::execute` stays synchronous for the design: the document, undo and the command
  reply never wait for, or depend on, a background result. Commands that read built
  solids (picking, measuring, naming a face for a new feature) need them up to date, so
  they first wait for the pending rebuild (or run it inline): results stay sequential.
- Only the UI uses the worker: after a command that changed the design, it sends the
  document (a clone; checkpoints are plain data and could be shared behind `Arc`) and a
  generation number to a worker that runs the same `rebuild` code; the viewport keeps the
  last good scene and shows a small "rebuilding..." label in the status bar. A newer
  generation makes the worker drop the older one's result (and can stop it between
  timeline items).
- Results come back as `(generation, Built, meshes)` and replace the shown solids only if
  the generation is still the latest; the caches merge the worker's checkpoints.
- The engine needs threads (not on the web build, which keeps the inline rebuild), and
  `Session` must become `Send` (it is, apart from the thread-local timers). With undo,
  redo and last-feature edits now under a millisecond, and the remaining cost in
  re-tessellating one changed solid, this was left out for now as not worth the risk.
