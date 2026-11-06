//! Making edits fast: the rebuild caches, and the rebuild timings (`debug.timings`).
//!
//! A rebuild walks the timeline in order. After each item it keeps a checkpoint: the
//! bodies built so far and everything else the walk carries
//! ([`crate::solids::Progress`]), under a key that hashes the item's definition (its
//! serialized form, whether it is suppressed, the reference signatures it keeps and the
//! inputs of every parameter its expressions name) together with the key of the item
//! before it. The next rebuild starts after the last item whose key it finds, so an edit
//! near the end of the timeline, or an undo or redo back to a design built before,
//! rebuilds only from the first item that changed. Because each key includes the one
//! before it, a changed item invalidates everything after it, exactly as a full rebuild
//! sees it, so the results are the same as a full rebuild's, bit for bit (the tests in
//! `perf_tests.rs` compare them).
//!
//! Meshes are cached too, by the exact body and the tessellation options, so solids an
//! edit did not change are not tessellated again.
//!
//! Both caches are bounded: once the bodies and meshes they hold pass their limit, the
//! least recently used entries go first.

use chamfera_doc::Document;
use chamfera_kernel::{Body, BodyMesh, KernelResult, TessOptions};
use serde::Serialize;
use serde_json::Value;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hasher};

/// How many bytes of checkpoints (bodies built so far) a session keeps by default.
pub const DEFAULT_CHECKPOINT_BYTES: usize = 256 * 1024 * 1024;
/// How many bytes of cached meshes a session keeps by default.
pub const DEFAULT_MESH_BYTES: usize = 128 * 1024 * 1024;
/// Most entries one cache holds, whatever their size.
pub const MAX_CACHE_ENTRIES: usize = 4096;
/// What every cache entry is counted as on top of its own size (the map's bookkeeping).
const ENTRY_OVERHEAD: usize = 256;

/// A 128-bit hash fed through [`std::io::Write`], so serde can stream into it. Two
/// SipHash states with different prefixes make an accidental collision practically
/// impossible (about one in 2^128).
pub(crate) struct KeyHasher {
    a: DefaultHasher,
    b: DefaultHasher,
}

impl KeyHasher {
    pub(crate) fn new() -> Self {
        let mut a = DefaultHasher::new();
        let mut b = DefaultHasher::new();
        a.write_u8(0xA5);
        b.write_u8(0x5A);
        b.write_u64(0x9E37_79B9_7F4A_7C15);
        KeyHasher { a, b }
    }
    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.a.write(bytes);
        self.b.write(bytes);
    }
    pub(crate) fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    pub(crate) fn u128(&mut self, v: u128) {
        self.bytes(&v.to_le_bytes());
    }
    pub(crate) fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.bytes(s.as_bytes());
    }
    /// Hashes `v`'s JSON form. `false` when it cannot be serialized.
    pub(crate) fn json(&mut self, v: &impl Serialize) -> bool {
        // A separator after the value keeps two values from running into each other.
        let ok = serde_json::to_writer(&mut *self, v).is_ok();
        self.bytes(&[0xFF]);
        ok
    }
    pub(crate) fn finish(&self) -> u128 {
        (u128::from(self.a.finish()) << 64) | u128::from(self.b.finish())
    }
}

impl std::io::Write for KeyHasher {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The inputs that decide a parameter's value, hashed: a user parameter's value, unit and
/// expression, a model parameter's expression or plain value, and those of every name
/// they refer to in turn.
/// What decides a model parameter: its expression, its plain value, its unit.
type ModelInputs = (Option<String>, Option<f64>, String);

struct ValueInputs<'a> {
    doc: &'a Document,
    /// Model parameters by the name expressions use: expression, plain value, unit.
    model: Option<BTreeMap<String, ModelInputs>>,
    /// Names already hashed (those whose hash did not depend on the path to them).
    memo: BTreeMap<String, u128>,
    /// Names looked at so far; past [`MAX_NAME_WORK`] the design is not cached.
    work: usize,
}

/// How deep [`ValueInputs`] follows names through other parameters' expressions; the
/// design's own evaluation refuses longer chains.
const MAX_NAME_DEPTH: usize = 64;
/// Most names [`ValueInputs`] looks at for one rebuild.
const MAX_NAME_WORK: usize = 200_000;

impl<'a> ValueInputs<'a> {
    fn new(doc: &'a Document) -> Self {
        ValueInputs {
            doc,
            model: None,
            memo: BTreeMap::new(),
            work: 0,
        }
    }

    fn exhausted(&self) -> bool {
        self.work > MAX_NAME_WORK
    }

    /// The hash of `name`'s inputs, and whether it depended on the path to it (a cycle or
    /// a chain too long, which evaluation refuses whatever the values).
    fn of(&mut self, name: &str, path: &mut Vec<String>) -> (u128, bool) {
        if let Some(h) = self.memo.get(name) {
            return (*h, false);
        }
        self.work = self.work.saturating_add(1);
        let mut h = KeyHasher::new();
        h.str(name);
        if self.exhausted() || path.iter().any(|p| p == name) || path.len() >= MAX_NAME_DEPTH {
            h.bytes(b"cycle");
            return (h.finish(), true);
        }
        let source = if let Some(p) = self.doc.parameters.get(name) {
            h.bytes(b"user");
            h.u64(p.value.to_bits());
            h.str(&format!("{:?}", p.unit));
            self.doc.effective_expression(name).map(str::to_owned)
        } else {
            let doc = self.doc;
            let model = self.model.get_or_insert_with(|| {
                doc.model_parameter_list()
                    .into_iter()
                    .map(|m| {
                        (
                            m.name,
                            (m.expression, m.plain_value, format!("{:?}", m.dim)),
                        )
                    })
                    .collect()
            });
            match model.get(name) {
                Some((expression, plain, dim)) => {
                    h.bytes(b"model");
                    h.u64(plain.map_or(u64::MAX, f64::to_bits));
                    h.str(dim);
                    expression.clone()
                }
                None => {
                    h.bytes(b"unknown");
                    None
                }
            }
        };
        let mut on_path = false;
        if let Some(src) = source {
            h.str(&src);
            path.push(name.to_owned());
            for dep in chamfera_doc::expr::names_in(&src) {
                let (d, p) = self.of(&dep, path);
                on_path |= p;
                h.u128(d);
            }
            path.pop();
        }
        let out = h.finish();
        if !on_path {
            self.memo.insert(name.to_owned(), out);
        }
        (out, on_path)
    }

    /// Hashes the inputs of every name any string in `v` refers to.
    fn hash_names(&mut self, v: &Value, h: &mut KeyHasher) {
        match v {
            Value::String(s) => {
                for name in chamfera_doc::expr::names_in(s) {
                    let (x, _) = self.of(&name, &mut Vec::new());
                    h.u128(x);
                }
            }
            Value::Array(a) => a.iter().for_each(|x| self.hash_names(x, h)),
            Value::Object(o) => o.values().for_each(|x| self.hash_names(x, h)),
            _ => {}
        }
    }
}

/// The checkpoint key after each of `items` (timeline order, up to the marker), or `None`
/// when the design cannot be hashed (then nothing is cached).
pub(crate) fn item_keys(doc: &Document, items: &[u32]) -> Option<Vec<u128>> {
    let mut values = ValueInputs::new(doc);
    // What every item may read besides itself: the unit expressions are written in, the
    // sheet metal rules, and the names of all items (error messages name them).
    let mut seed = KeyHasher::new();
    seed.bytes(b"chamfera rebuild v1");
    seed.u64(doc.length_scale().to_bits());
    let rules = serde_json::to_value(&doc.sheet_rules).ok()?;
    seed.json(&rules).then_some(())?;
    values.hash_names(&rules, &mut seed);
    for s in &doc.sketches {
        seed.u64(u64::from(s.id));
        seed.str(&s.name);
    }
    for f in &doc.features {
        seed.u64(u64::from(f.id));
        seed.str(&f.name);
    }
    let mut prev = seed.finish();
    let mut keys = Vec::with_capacity(items.len());
    for &id in items {
        let mut h = KeyHasher::new();
        h.u128(prev);
        h.u64(u64::from(id));
        let item = if let Ok(s) = doc.sketch(id) {
            h.bytes(b"sketch");
            serde_json::to_value(s).ok()?
        } else if let Ok(f) = doc.feature(id) {
            h.bytes(b"feature");
            h.bytes(&[u8::from(doc.is_suppressed(f))]);
            serde_json::to_value(f).ok()?
        } else {
            h.bytes(b"none");
            Value::Null
        };
        h.json(&item).then_some(())?;
        values.hash_names(&item, &mut h);
        if let Some(r) = doc.references.get(&id) {
            h.json(r).then_some(())?;
        }
        prev = h.finish();
        keys.push(prev);
    }
    (!values.exhausted()).then_some(keys)
}

/// A map with a byte limit that forgets the least recently used entries first.
#[derive(Debug)]
pub(crate) struct Lru<V> {
    entries: BTreeMap<u128, Entry<V>>,
    bytes: usize,
    limit: usize,
    tick: u64,
}

#[derive(Debug)]
struct Entry<V> {
    value: V,
    bytes: usize,
    used: u64,
}

impl<V> Lru<V> {
    pub(crate) fn new(limit: usize) -> Self {
        Lru {
            entries: BTreeMap::new(),
            bytes: 0,
            limit,
            tick: 0,
        }
    }

    pub(crate) fn contains(&self, key: &u128) -> bool {
        self.entries.contains_key(key)
    }

    /// The entry under `key`, marked as just used.
    pub(crate) fn get(&mut self, key: &u128) -> Option<&V> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        let e = self.entries.get_mut(key)?;
        e.used = tick;
        Some(&e.value)
    }

    fn remove(&mut self, key: &u128) {
        if let Some(e) = self.entries.remove(key) {
            self.bytes = self.bytes.saturating_sub(e.bytes);
        }
    }

    /// Keeps `value` (about `bytes` big). A value larger than the whole limit is not kept.
    pub(crate) fn insert(&mut self, key: u128, value: V, bytes: usize) {
        self.remove(&key);
        let bytes = bytes.saturating_add(ENTRY_OVERHEAD);
        if bytes > self.limit {
            return;
        }
        while self.bytes.saturating_add(bytes) > self.limit
            || self.entries.len() >= MAX_CACHE_ENTRIES
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| *k)
            else {
                break;
            };
            self.remove(&oldest);
        }
        self.tick = self.tick.wrapping_add(1);
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(
            key,
            Entry {
                value,
                bytes,
                used: self.tick,
            },
        );
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    /// Changes the limit, forgetting entries until the cache fits.
    pub(crate) fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        while self.bytes > self.limit {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| *k)
            else {
                break;
            };
            self.remove(&oldest);
        }
    }
}

/// About how many bytes a mesh holds.
fn mesh_bytes(m: &BodyMesh) -> usize {
    let points: usize = m.edge_polylines.iter().map(|(_, l)| l.len()).sum();
    m.mesh
        .positions
        .len()
        .saturating_add(points)
        .saturating_mul(24)
        .saturating_add(m.mesh.triangles.len().saturating_mul(12 + 4))
        .saturating_add(m.edge_polylines.len().saturating_mul(32))
}

/// A cheap key for a body's mesh: its entity counts and vertex positions, and the
/// options. Two bodies with the same key are told apart by comparing them exactly.
fn mesh_key(body: &Body, options: &TessOptions) -> u128 {
    let mut h = KeyHasher::new();
    h.u64(options.chord_tolerance.to_bits());
    h.u64(options.angle_tolerance.to_bits());
    h.u64(options.max_triangles as u64);
    let s = body.summary();
    for n in [
        s.shells,
        s.faces,
        s.loops,
        s.edges,
        s.half_edges,
        s.vertices,
    ] {
        h.u64(n as u64);
    }
    for v in body.vertices() {
        if let Ok(p) = body.vertex_point(v) {
            h.u64(p.x.to_bits());
            h.u64(p.y.to_bits());
            h.u64(p.z.to_bits());
        }
    }
    h.finish()
}

/// How often the caches were used since the session started (or [`Caches::clear`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CacheStats {
    /// Timeline items whose checkpoint was reused by the last rebuild.
    pub last_reused: usize,
    /// Timeline items the last rebuild built.
    pub last_built: usize,
    pub mesh_hits: usize,
    pub mesh_misses: usize,
}

/// The session's rebuild caches.
#[derive(Debug)]
pub(crate) struct Caches {
    /// Off: every rebuild builds the whole timeline (to compare, and to time it).
    pub(crate) enabled: bool,
    pub(crate) checkpoints: Lru<crate::solids::Progress>,
    pub(crate) meshes: Lru<(Body, BodyMesh)>,
    pub(crate) stats: CacheStats,
}

impl Default for Caches {
    fn default() -> Self {
        Caches {
            enabled: true,
            checkpoints: Lru::new(DEFAULT_CHECKPOINT_BYTES),
            meshes: Lru::new(DEFAULT_MESH_BYTES),
            stats: CacheStats::default(),
        }
    }
}

impl Caches {
    pub(crate) fn clear(&mut self) {
        self.checkpoints.clear();
        self.meshes.clear();
        self.stats = CacheStats::default();
    }

    /// Tessellates `body`, or returns the mesh made last time for an equal body.
    pub(crate) fn tessellate(
        &mut self,
        body: &Body,
        options: &TessOptions,
    ) -> KernelResult<BodyMesh> {
        if !self.enabled {
            return chamfera_kernel::tessellate(body, options);
        }
        let key = mesh_key(body, options);
        if let Some((b, m)) = self.meshes.get(&key)
            && b == body
        {
            self.stats.mesh_hits = self.stats.mesh_hits.saturating_add(1);
            return Ok(m.clone());
        }
        self.stats.mesh_misses = self.stats.mesh_misses.saturating_add(1);
        let m = chamfera_kernel::tessellate(body, options)?;
        let bytes = body.approx_bytes().saturating_add(mesh_bytes(&m));
        self.meshes.insert(key, (body.clone(), m.clone()), bytes);
        Ok(m)
    }
}

impl crate::Session {
    /// Switches the rebuild caches on or off. Off forgets everything they held, and every
    /// rebuild then builds the whole timeline.
    pub fn set_rebuild_cache(&mut self, enabled: bool) {
        self.caches.enabled = enabled;
        if !enabled {
            self.caches.clear();
        }
    }

    /// Forgets the checkpoints and meshes of earlier rebuilds.
    pub fn clear_rebuild_cache(&mut self) {
        self.caches.clear();
    }

    /// Changes the caches' memory limits (bytes of checkpoints, bytes of meshes).
    pub fn set_rebuild_cache_limits(&mut self, checkpoints: usize, meshes: usize) {
        self.caches.checkpoints.set_limit(checkpoints);
        self.caches.meshes.set_limit(meshes);
    }

    /// How many bytes the caches hold now (checkpoints, meshes) and how many entries.
    pub fn rebuild_cache_usage(&self) -> CacheUsage {
        CacheUsage {
            checkpoint_bytes: self.caches.checkpoints.bytes(),
            checkpoint_limit: self.caches.checkpoints.limit(),
            checkpoints: self.caches.checkpoints.len(),
            mesh_bytes: self.caches.meshes.bytes(),
            mesh_limit: self.caches.meshes.limit(),
            meshes: self.caches.meshes.len(),
            stats: self.caches.stats,
        }
    }
}

/// What the rebuild caches hold.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CacheUsage {
    pub checkpoint_bytes: usize,
    pub checkpoint_limit: usize,
    pub checkpoints: usize,
    pub mesh_bytes: usize,
    pub mesh_limit: usize,
    pub meshes: usize,
    pub stats: CacheStats,
}

// ---- Timings ----------------------------------------------------------------------

/// Where a feature's rebuild time went, in milliseconds.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct FeatureTiming {
    pub id: u32,
    pub name: String,
    /// The feature kind (`extrude`, `blend`), or `sketch`.
    pub kind: String,
    /// Building the shapes, less the booleans and naming below.
    pub kernel_ms: f64,
    /// Joining, cutting and intersecting with the bodies built so far.
    pub booleans_ms: f64,
    /// Naming the new faces and edges, and finding the named ones it refers to.
    pub naming_ms: f64,
    /// Meshing the solids it made, for the viewport.
    pub tessellation_ms: f64,
    pub total_ms: f64,
}

/// What a phase timer adds to.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Phase {
    Booleans,
    Naming,
}

thread_local! {
    static TIMING: Cell<bool> = const { Cell::new(false) };
    static BOOLEANS: Cell<f64> = const { Cell::new(0.0) };
    static NAMING: Cell<f64> = const { Cell::new(0.0) };
}

/// A moment to measure from; `None` when timing is off or there is no clock (the web
/// build has none, and asking for the time there would abort).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Stopwatch(Option<std::time::Instant>);

impl Stopwatch {
    pub(crate) fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        if TIMING.get() {
            return Stopwatch(Some(std::time::Instant::now()));
        }
        Stopwatch(None)
    }
    /// Milliseconds since the start (0 without a clock).
    pub(crate) fn ms(self) -> f64 {
        self.0.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0)
    }
}

/// True when this build can time rebuilds.
pub(crate) fn has_clock() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

pub(crate) fn set_timing(on: bool) {
    TIMING.set(on);
}

pub(crate) fn timing() -> bool {
    TIMING.get()
}

/// Runs `f`, adding its time to `phase` when timing is on.
pub(crate) fn timed<T>(phase: Phase, f: impl FnOnce() -> T) -> T {
    if !TIMING.get() {
        return f();
    }
    let t = Stopwatch::start();
    let out = f();
    let ms = t.ms();
    let cell = match phase {
        Phase::Booleans => &BOOLEANS,
        Phase::Naming => &NAMING,
    };
    cell.with(|c| c.set(c.get() + ms));
    out
}

/// Runs a boolean, timing it as [`Phase::Booleans`].
pub(crate) fn boolean<T>(f: impl FnOnce() -> T) -> T {
    timed(Phase::Booleans, f)
}

/// Runs naming work, timing it as [`Phase::Naming`].
pub(crate) fn naming<T>(f: impl FnOnce() -> T) -> T {
    timed(Phase::Naming, f)
}

/// Booleans and naming time so far, and resets both.
pub(crate) fn take_phases() -> (f64, f64) {
    (BOOLEANS.replace(0.0), NAMING.replace(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_lru_forgets_the_least_recently_used_entry_first() {
        // Each entry counts as its size plus ENTRY_OVERHEAD, so three of 100 fit in
        // 3 * (100 + 256) = 1068 bytes and a fourth does not.
        let mut c: Lru<u32> = Lru::new(3 * (100 + ENTRY_OVERHEAD));
        c.insert(1, 1, 100);
        c.insert(2, 2, 100);
        c.insert(3, 3, 100);
        assert_eq!(c.get(&1), Some(&1)); // 2 is now the oldest
        c.insert(4, 4, 100);
        assert!(c.contains(&1) && !c.contains(&2) && c.contains(&3) && c.contains(&4));
        assert!(c.bytes() <= c.limit());
        // Larger than the whole limit: not kept, nothing else lost.
        c.insert(5, 5, 10_000);
        assert!(!c.contains(&5));
        assert_eq!(c.len(), 3);
        c.set_limit(100 + ENTRY_OVERHEAD);
        assert_eq!(c.len(), 1);
        assert!(c.contains(&4));
    }

    #[test]
    fn keys_differ_when_any_input_differs() {
        let key = |f: &dyn Fn(&mut KeyHasher)| {
            let mut h = KeyHasher::new();
            f(&mut h);
            h.finish()
        };
        let a = key(&|h| h.str("ab"));
        assert_eq!(a, key(&|h| h.str("ab")));
        assert_ne!(a, key(&|h| h.str("ba")));
        // Length-prefixed: "a" + "b" is not "ab".
        assert_ne!(
            key(&|h| {
                h.str("a");
                h.str("b")
            }),
            a
        );
    }
}
