//! The toolpath cache and background generation.
//!
//! Toolpaths take seconds for the 3D operations, and a rebuild works them all out again,
//! so each operation's result is kept under a 128-bit key of everything it is made from:
//! - the operation's definition, its tool and its setup (their JSON);
//! - the setup's geometry as worked out from the model (stock box, model box, work
//!   coordinates and clearance, bit for bit), which every operation reads;
//! - the geometry it machines:
//!   - facing: nothing more (it reads only the stock and the model's box);
//!   - the 3D operations and drilling every hole: the whole of every body (a hash of its
//!     exact data, each coordinate, curve and surface, bit for bit);
//!   - contours, pockets and drilling named holes: the persistent names they refer to,
//!     resolved through K8 naming (which body each resolves to, or why it does not), the
//!     exact geometry of the faces and edges they resolve to (name, surface, loops in
//!     order, edges, vertices and the faces across each edge, without ids, which a
//!     rebuild may renumber), and only what else they can read: a pocket its walls (the
//!     faces across the floor's edges) whole and, of every other face of the body, the
//!     edges whose boxes meet the floor's box in plan (the points it checks for
//!     overhangs, vertices and edge middles, lie on them); named holes every face whose
//!     box (`chamfera_cam::drop::face_box`, holding every point of it) meets theirs and
//!     whose surface is a cylinder (a coaxial one's box holds the axis); a contour
//!     nothing more. A feature far from them changes none of that, so the toolpath is
//!     reused.
//!
//! A rebuild finds each operation's key; an operation whose key the cache holds gets that
//! toolpath, every other is generated again. The cache is an LRU with a byte limit
//! ([`DEFAULT_TOOLPATH_BYTES`]), so undoing back to a design generated before is instant.
//! It must never change a toolpath: `perf_tests.rs` compares a cached session against
//! one generating everything after every step of every example and of random edits.
//!
//! **Background generation** is the front end's choice ([`Session::set_cam_background`]):
//! then a rebuild only marks the operations it could not take from the cache as stale
//! (their old toolpaths stay, drawn dimmed) and leaves a [`CamJob`] for the front end to
//! run on a worker thread ([`CamJob::run`], a pure function of the snapshot it holds).
//! [`Session::accept_cam_result`] puts the results in the cache and shows those whose
//! key is still current; a newer job cancels an older one ([`chamfera_cam::cancel`]).
//! Commands that read toolpaths (`cam.generate`, `cam.post`, `cam.simulate`, `cam.list`,
//! adding or editing an operation or a tool) first generate whatever is stale, so their replies
//! never depend on the worker. Scripts, tests and agents never turn background
//! generation on.

use crate::Session;
use crate::perf::{KeyHasher, Lru};
use chamfera_cam::Toolpath;
use chamfera_doc::cam::{Cam, CamOperation, OpKind};
use chamfera_doc::naming::{self, Kind, Want};
use chamfera_kernel::{Body, FaceId};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::Arc;

/// How many bytes of toolpaths the cache keeps by default.
pub const DEFAULT_TOOLPATH_BYTES: usize = 64 * 1024 * 1024;

/// A toolpath result: the toolpath, or why there is none.
pub type PathResult = Result<Toolpath, String>;

/// Feeds `Debug` text into the hasher (exact: `f64` prints its shortest round-trip form).
struct Text<'a>(&'a mut KeyHasher);

impl std::fmt::Write for Text<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.bytes(s.as_bytes());
        Ok(())
    }
}

fn debug(h: &mut KeyHasher, v: &impl std::fmt::Debug) {
    let _ = write!(Text(h), "{v:?}");
    h.bytes(&[0xFE]);
}

/// Hashes of whole bodies, worked out once per rebuild and only if needed.
pub(crate) struct BodyHashes<'a> {
    bodies: &'a [Body],
    hashes: Vec<Option<u128>>,
}

impl<'a> BodyHashes<'a> {
    pub(crate) fn new(bodies: &'a [Body]) -> Self {
        BodyHashes {
            bodies,
            hashes: vec![None; bodies.len()],
        }
    }

    fn all(&mut self) -> Vec<u128> {
        (0..self.bodies.len()).map(|i| self.of(i)).collect()
    }

    fn of(&mut self, i: usize) -> u128 {
        if let Some(Some(h)) = self.hashes.get(i) {
            return *h;
        }
        let mut h = KeyHasher::new();
        match self.bodies.get(i) {
            Some(b) => {
                if !h.json(b) {
                    debug(&mut h, b);
                }
            }
            None => h.bytes(b"none"),
        }
        let out = h.finish();
        if let Some(slot) = self.hashes.get_mut(i) {
            *slot = Some(out);
        }
        out
    }
}

/// The persistent names an operation refers to, and of what kind.
fn references(op: &OpKind) -> Option<Vec<(Kind, &str, Want)>> {
    match op {
        OpKind::Contour { face, edges, .. } => Some(match face {
            Some(f) => vec![(Kind::Face, f.as_str(), Want::One)],
            None => edges
                .iter()
                .map(|e| (Kind::Edge, e.as_str(), Want::One))
                .collect(),
        }),
        OpKind::Pocket { face, .. } => Some(vec![(Kind::Face, face.as_str(), Want::One)]),
        OpKind::Drill { faces, .. } if !faces.is_empty() => Some(
            faces
                .iter()
                .map(|f| (Kind::Face, f.as_str(), Want::All))
                .collect(),
        ),
        _ => None,
    }
}

/// Hashes a face exactly, without its ids (a rebuild may number the same face
/// differently): its name, surface, and every loop's edges in order (names, curves, ends,
/// direction) and the name and surface of the face across each edge.
fn hash_face(h: &mut KeyHasher, body: &Body, f: FaceId) {
    h.bytes(b"face");
    debug(h, &body.face_name(f));
    debug(h, &body.face_surface(f));
    let outer = body.face_outer_loop(f).ok();
    for lp in body.face_loops(f).unwrap_or_default() {
        h.bytes(if Some(lp) == outer {
            b"outer"
        } else {
            b"inner"
        });
        for he in body.loop_half_edges(lp).unwrap_or_default() {
            match body.half_edge_edge(he).ok().flatten() {
                Some(e) => {
                    hash_edge(h, body, e);
                    debug(h, &body.edge_half_edges(e).ok().map(|x| x[0] == he));
                }
                None => h.bytes(b"no edge"),
            }
            debug(
                h,
                &body.half_edge_origin(he).ok().map(|v| body.vertex_point(v)),
            );
            debug(
                h,
                &body.half_edge_end(he).ok().map(|v| body.vertex_point(v)),
            );
            match across(body, he) {
                Some(g) => {
                    debug(h, &body.face_name(g));
                    debug(h, &body.face_surface(g));
                }
                None => h.bytes(b"open"),
            }
        }
    }
}

/// The face across a half-edge.
fn across(body: &Body, he: chamfera_kernel::HalfEdgeId) -> Option<FaceId> {
    body.half_edge_mate(he)
        .ok()
        .flatten()
        .and_then(|m| body.half_edge_loop(m).ok())
        .and_then(|l| body.loop_face(l).ok())
}

/// Hashes an edge exactly, without its id: its name, curve and both half-edges' ends.
fn hash_edge(h: &mut KeyHasher, body: &Body, e: chamfera_kernel::EdgeId) {
    h.bytes(b"edge");
    debug(h, &naming::edge_ref(body, e));
    debug(h, &body.edge_curve(e));
    if let Ok(hs) = body.edge_half_edges(e) {
        for he in hs {
            debug(
                h,
                &body.half_edge_origin(he).ok().map(|v| body.vertex_point(v)),
            );
            debug(
                h,
                &body.half_edge_end(he).ok().map(|v| body.vertex_point(v)),
            );
        }
    }
}

/// The XY box of an edge's curve, if it has one.
fn edge_box(body: &Body, e: chamfera_kernel::EdgeId) -> Option<chamfera_geom::Aabb> {
    let (c, t0, t1) = body.edge_curve(e).ok().flatten()?;
    c.bounding_box_on(t0, t1).ok()
}

fn meets(a: &chamfera_geom::Aabb, b: &chamfera_geom::Aabb) -> bool {
    let slack = 1e-6;
    a.min.x <= b.max.x + slack
        && b.min.x <= a.max.x + slack
        && a.min.y <= b.max.y + slack
        && b.min.y <= a.max.y + slack
}

/// The key of an operation's toolpath (see the module notes for what it holds).
pub(crate) fn op_key(cam: &Cam, op: &CamOperation, bodies: &[Body], bh: &mut BodyHashes) -> u128 {
    let mut h = KeyHasher::new();
    h.bytes(b"chamfera toolpath v1");
    if !h.json(op) {
        debug(&mut h, op);
    }
    let setup = cam.setup(op.setup);
    let tool = cam.tool(op.tool);
    if !h.json(&setup) {
        debug(&mut h, &setup);
    }
    if !h.json(&tool) {
        debug(&mut h, &tool);
    }
    if op.suppressed {
        return h.finish();
    }
    let (Some(setup), Some(_)) = (setup, tool) else {
        return h.finish();
    };
    match chamfera_cam::setup_geometry(setup, bodies, crate::solids::edge_ref) {
        Ok(g) => {
            for p in [
                g.stock_min,
                g.stock_max,
                g.model_min,
                g.model_max,
                g.wcs.origin,
            ] {
                h.u64(p.x.to_bits());
                h.u64(p.y.to_bits());
                h.u64(p.z.to_bits());
            }
            h.u64(g.wcs.angle.to_bits());
            h.u64(g.clearance_z.to_bits());
        }
        Err(e) => {
            h.str(&e.to_string());
            return h.finish();
        }
    }
    if matches!(op.op, OpKind::Facing { .. }) {
        return h.finish();
    }
    let Some(refs) = references(&op.op) else {
        // The 3D operations and drilling every hole read the whole model.
        h.bytes(b"whole");
        for b in bh.all() {
            h.u128(b);
        }
        return h.finish();
    };
    let list: Vec<&Body> = bodies.iter().collect();
    // Per body: the faces referenced, and the boxes in plan the operation reaches.
    let mut reach: BTreeMap<usize, (Vec<FaceId>, Vec<chamfera_geom::Aabb>)> = BTreeMap::new();
    for (kind, name, want) in refs {
        h.str(name);
        match naming::resolve(&list, kind, name, None, want) {
            Ok(found) => {
                for f in found {
                    h.u64(f.body as u64);
                    let Some(body) = bodies.get(f.body) else {
                        continue;
                    };
                    let slot = reach.entry(f.body).or_default();
                    if let Some(face) = f.face() {
                        hash_face(&mut h, body, face);
                        slot.0.push(face);
                        // Unknown box: the whole body is in reach.
                        let b = chamfera_cam::drop::face_box(body, face)
                            .ok()
                            .flatten()
                            .or_else(|| body.bounding_box());
                        slot.1.extend(b);
                    }
                    if let Some(e) = f.edge() {
                        hash_edge(&mut h, body, e);
                        slot.1.extend(edge_box(body, e));
                    }
                }
            }
            Err(e) => h.str(&e.to_string()),
        }
    }
    for (bi, (faces, boxes)) in &reach {
        let Some(body) = bodies.get(*bi) else {
            continue;
        };
        h.bytes(b"body");
        h.u64(*bi as u64);
        match &op.op {
            OpKind::Pocket { .. } => {
                // The walls (across the floor's edges) whole: their kind and height.
                let mut walls: BTreeSet<FaceId> = BTreeSet::new();
                for f in faces {
                    walls.insert(*f);
                    for lp in body.face_loops(*f).unwrap_or_default() {
                        for he in body.loop_half_edges(lp).unwrap_or_default() {
                            if let Some(g) = across(body, he) {
                                walls.insert(g);
                            }
                        }
                    }
                }
                for f in body.faces() {
                    if walls.contains(&f) {
                        if !faces.contains(&f) {
                            hash_face(&mut h, body, f);
                        }
                        continue;
                    }
                    // Any other face is read only for its points over the floor: the
                    // ends and middles of its edges, each inside its edge's box.
                    let mut first = true;
                    for e in body.face_edges(f).unwrap_or_default() {
                        if edge_box(body, e).is_none_or(|b| boxes.iter().any(|r| meets(r, &b))) {
                            if first {
                                h.bytes(b"near");
                                debug(&mut h, &body.face_name(f));
                                first = false;
                            }
                            hash_edge(&mut h, body, e);
                        }
                    }
                }
            }
            OpKind::Drill { .. } => {
                // Every coaxial cylinder (each one's box holds the axis) whole; other
                // faces in reach only for what kind of surface they are.
                for f in body.faces() {
                    let near = match chamfera_cam::drop::face_box(body, f) {
                        Ok(Some(fb)) => boxes.iter().any(|b| meets(b, &fb)),
                        _ => true,
                    };
                    if !near {
                        continue;
                    }
                    let cylinder = matches!(
                        body.face_surface(f),
                        Ok(Some((chamfera_kernel::Surface::Cylinder(_), _)))
                    );
                    if cylinder {
                        hash_face(&mut h, body, f);
                    } else {
                        h.bytes(b"not a cylinder");
                    }
                }
            }
            // A contour reads only its face or edges, hashed above.
            _ => {}
        }
    }
    h.finish()
}

/// One operation's toolpath for a snapshot of the design, or `None` when the job was
/// cancelled while making it.
fn generate_one(cam: &Cam, op: &CamOperation, bodies: &[Body]) -> Option<PathResult> {
    if op.suppressed {
        return Some(Err("suppressed".to_owned()));
    }
    match chamfera_cam::generate(cam, op, bodies, crate::solids::edge_ref) {
        Ok(tp) => Some(Ok(tp)),
        Err(chamfera_cam::CamError::Cancelled) => None,
        Err(e) => Some(Err(e.to_string())),
    }
}

/// Every operation's toolpath for a snapshot of the design: the pure function background
/// generation is built on (the same results as the session's own, cache or not).
pub fn generate_snapshot(cam: &Cam, bodies: &[Body]) -> BTreeMap<u32, PathResult> {
    cam.operations
        .iter()
        .filter_map(|op| Some((op.id, generate_one(cam, op, bodies)?)))
        .collect()
}

/// About how many bytes a toolpath result holds.
fn result_bytes(r: &PathResult) -> usize {
    match r {
        Ok(tp) => tp
            .moves
            .len()
            .saturating_mul(std::mem::size_of::<chamfera_cam::Move>())
            .saturating_add(tp.tool_name.len()),
        Err(e) => e.len(),
    }
}

/// The toolpaths a rebuild could not take from the cache, to make on a worker thread.
#[derive(Debug, Clone)]
pub struct CamJob {
    /// The session's toolpath generation when the job was made; a newer one cancels it.
    pub generation: u64,
    cam: Cam,
    bodies: Arc<Vec<Body>>,
    /// The operations to make, and the key each result is for.
    todo: Vec<(u32, u128)>,
}

/// What a [`CamJob`] made.
#[derive(Debug, Clone)]
pub struct CamJobResult {
    pub generation: u64,
    results: Vec<(u32, u128, PathResult)>,
    /// Some operations were not made: a newer job was asked for.
    pub cancelled: bool,
}

impl CamJob {
    /// Makes the job's toolpaths (pure: the same as the session would make). Run inside
    /// `chamfera_cam::cancel::with` on a worker so a newer job stops it.
    pub fn run(&self) -> CamJobResult {
        let mut results = Vec::new();
        let mut cancelled = false;
        for (id, key) in &self.todo {
            let Some(op) = self.cam.operation(*id) else {
                continue;
            };
            match generate_one(&self.cam, op, &self.bodies) {
                Some(r) => results.push((*id, *key, r)),
                None => {
                    cancelled = true;
                    break;
                }
            }
        }
        CamJobResult {
            generation: self.generation,
            results,
            cancelled,
        }
    }

    /// [`CamJob::run`] on a worker: it stops (its result says `cancelled`) once `latest`
    /// holds a newer generation than this job's.
    pub fn run_unless_newer(&self, latest: Arc<std::sync::atomic::AtomicU64>) -> CamJobResult {
        let token = chamfera_cam::cancel::Token {
            latest,
            generation: self.generation,
        };
        if token.cancelled() {
            return CamJobResult {
                generation: self.generation,
                results: Vec::new(),
                cancelled: true,
            };
        }
        chamfera_cam::cancel::with(token, || self.run())
    }

    /// How many operations it makes.
    pub fn len(&self) -> usize {
        self.todo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.todo.is_empty()
    }
}

/// How the toolpath cache was used.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ToolpathCacheStats {
    /// Operations of the last regeneration taken from the cache.
    pub last_hits: usize,
    /// Operations of the last regeneration made again (or left to the worker).
    pub last_misses: usize,
    pub hits: usize,
    pub misses: usize,
    pub entries: usize,
    pub bytes: usize,
}

/// The session's toolpath cache and background state.
#[derive(Debug)]
pub(crate) struct ToolpathCache {
    pub(crate) enabled: bool,
    lru: Lru<PathResult>,
    /// The key each operation's toolpath has to have now.
    keys: BTreeMap<u32, u128>,
    /// Operations whose shown toolpath is older than their key (or missing).
    stale: BTreeSet<u32>,
    pub(crate) background: bool,
    generation: u64,
    job: Option<CamJob>,
    stats: ToolpathCacheStats,
}

impl Default for ToolpathCache {
    fn default() -> Self {
        ToolpathCache {
            enabled: true,
            lru: Lru::new(DEFAULT_TOOLPATH_BYTES),
            keys: BTreeMap::new(),
            stale: BTreeSet::new(),
            background: false,
            generation: 0,
            job: None,
            stats: ToolpathCacheStats::default(),
        }
    }
}

impl Session {
    /// Works out every operation's toolpath from the model as it is now: from the cache
    /// where the key matches, else made again (or, with background generation on, left
    /// stale for the worker).
    pub fn regenerate_cam(&mut self) {
        // The model or the operations changed: an old simulation no longer holds.
        self.clear_simulation();
        let tc = &mut self.cam.tc;
        tc.generation = tc.generation.wrapping_add(1);
        tc.job = None;
        tc.stats.last_hits = 0;
        tc.stats.last_misses = 0;
        if self.doc.cam.operations.is_empty() {
            self.cam.paths.clear();
            tc.keys.clear();
            tc.stale.clear();
            return;
        }
        let bodies: Vec<Body> = self.built.solids.iter().map(|s| s.body.clone()).collect();
        let mut bh = BodyHashes::new(&bodies);
        let mut keys = BTreeMap::new();
        let mut todo = Vec::new();
        let mut paths = std::mem::take(&mut self.cam.paths);
        paths.retain(|id, _| self.doc.cam.operation(*id).is_some());
        let tc = &mut self.cam.tc;
        tc.stale.clear();
        for op in &self.doc.cam.operations {
            let key = op_key(&self.doc.cam, op, &bodies, &mut bh);
            keys.insert(op.id, key);
            let hit = if tc.enabled {
                tc.lru.get(&key).cloned()
            } else {
                None
            };
            if let Some(r) = hit {
                tc.stats.last_hits += 1;
                paths.insert(op.id, r);
                continue;
            }
            tc.stats.last_misses += 1;
            if tc.background {
                tc.stale.insert(op.id);
                todo.push((op.id, key));
                continue;
            }
            let r = generate_one(&self.doc.cam, op, &bodies)
                .unwrap_or_else(|| Err(chamfera_cam::CamError::Cancelled.to_string()));
            if tc.enabled {
                let bytes = result_bytes(&r);
                tc.lru.insert(key, r.clone(), bytes);
            }
            paths.insert(op.id, r);
        }
        tc.stats.hits += tc.stats.last_hits;
        tc.stats.misses += tc.stats.last_misses;
        tc.keys = keys;
        if !todo.is_empty() {
            tc.job = Some(CamJob {
                generation: tc.generation,
                cam: self.doc.cam.clone(),
                bodies: Arc::new(bodies),
                todo,
            });
        }
        self.cam.paths = paths;
    }

    /// Makes every stale toolpath now (on this thread), so what follows reads the current
    /// ones. Does nothing when none is stale.
    pub fn ensure_cam_current(&mut self) {
        if self.cam.tc.stale.is_empty() {
            return;
        }
        let bodies: Vec<Body> = self.built.solids.iter().map(|s| s.body.clone()).collect();
        let tc = &mut self.cam.tc;
        // Any job out on a worker is no longer needed.
        tc.generation = tc.generation.wrapping_add(1);
        tc.job = None;
        let stale: Vec<u32> = std::mem::take(&mut tc.stale).into_iter().collect();
        for id in stale {
            let Some(op) = self.doc.cam.operation(id) else {
                self.cam.paths.remove(&id);
                continue;
            };
            let Some(key) = self.cam.tc.keys.get(&id).copied() else {
                continue;
            };
            let r = generate_one(&self.doc.cam, op, &bodies)
                .unwrap_or_else(|| Err(chamfera_cam::CamError::Cancelled.to_string()));
            if self.cam.tc.enabled {
                let bytes = result_bytes(&r);
                self.cam.tc.lru.insert(key, r.clone(), bytes);
            }
            self.cam.paths.insert(id, r);
        }
        self.refresh_overlays();
    }

    /// Regenerates and makes every stale toolpath now: what commands that read toolpaths
    /// use, so their replies never depend on a worker.
    pub fn regenerate_cam_now(&mut self) {
        self.regenerate_cam();
        self.ensure_cam_current();
    }

    /// Background generation on (the desktop app) or off (scripts, tests, agents).
    pub fn set_cam_background(&mut self, on: bool) {
        self.cam.tc.background = on;
        if !on {
            self.ensure_cam_current();
        }
    }

    /// The job a rebuild left for a worker, if any (taken: asked once).
    pub fn take_cam_job(&mut self) -> Option<CamJob> {
        self.cam.tc.job.take()
    }

    /// The latest toolpath generation: a worker's job of an older one is cancelled.
    pub fn cam_generation(&self) -> u64 {
        self.cam.tc.generation
    }

    /// True while some toolpath is stale (being made on the worker).
    pub fn cam_regenerating(&self) -> bool {
        !self.cam.tc.stale.is_empty()
    }

    /// True when operation `id`'s shown toolpath is stale.
    pub fn toolpath_stale(&self, id: u32) -> bool {
        self.cam.tc.stale.contains(&id)
    }

    /// Takes a worker's results: each goes into the cache, and replaces the shown
    /// toolpath when it is for that operation's current key. True when one did.
    pub fn accept_cam_result(&mut self, r: CamJobResult) -> bool {
        let mut changed = false;
        for (id, key, res) in r.results {
            let tc = &mut self.cam.tc;
            if tc.enabled {
                let bytes = result_bytes(&res);
                tc.lru.insert(key, res.clone(), bytes);
            }
            if tc.keys.get(&id) == Some(&key) && tc.stale.remove(&id) {
                self.cam.paths.insert(id, res);
                changed = true;
            }
        }
        if changed {
            self.refresh_overlays();
        }
        changed
    }

    /// Switches the toolpath cache on or off (off forgets what it held).
    pub fn set_toolpath_cache(&mut self, enabled: bool) {
        self.cam.tc.enabled = enabled;
        if !enabled {
            self.cam.tc.lru.clear();
        }
    }

    /// Changes the toolpath cache's byte limit.
    pub fn set_toolpath_cache_limit(&mut self, bytes: usize) {
        self.cam.tc.lru.set_limit(bytes);
    }

    /// How the toolpath cache was used, and what it holds.
    pub fn toolpath_cache_stats(&self) -> ToolpathCacheStats {
        ToolpathCacheStats {
            entries: self.cam.tc.lru.len(),
            bytes: self.cam.tc.lru.bytes(),
            ..self.cam.tc.stats
        }
    }
}
