//! Incremental rebuilds give the same result as building everything.
//!
//! Every test runs two sessions side by side on the same commands: one with the rebuild
//! cache (checkpoints and meshes, `perf.rs`), one without it, which builds the whole
//! timeline after every change as Chamfera always did. After every command the two must
//! agree exactly: the same bodies bit for bit (so the same volumes, faces and names), the
//! same meshes, errors, sketches and toolpaths.

use super::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Rewrites the scripts' relative paths: `target/qa/` into `out`, `examples/` to the repo's.
fn fix_paths(v: &Value, out: &Path) -> Value {
    match v {
        Value::String(s) if s.starts_with("target/qa/") => {
            json!(out.join(&s["target/qa/".len()..]).to_string_lossy())
        }
        Value::String(s) if s.starts_with("examples/") => {
            json!(repo().join(s).to_string_lossy())
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| fix_paths(x, out)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), fix_paths(x, out)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// A session with the rebuild cache and one without.
struct Twins {
    cached: Session,
    full: Session,
    /// Steps compared so far.
    steps: usize,
}

impl Twins {
    fn new() -> Self {
        let mut full = Session::new();
        full.set_rebuild_cache(false);
        Twins {
            cached: Session::new(),
            full,
            steps: 0,
        }
    }

    /// Runs the command on both; both must succeed or fail alike and then agree.
    fn run(&mut self, id: &str, p: &Value, what: &str) -> bool {
        let a = self.cached.execute(id, p);
        let b = self.full.execute(id, p);
        assert_eq!(
            a.as_ref().map_err(ToString::to_string),
            b.as_ref().map_err(ToString::to_string),
            "{what}: {id} {p}"
        );
        self.check(&format!("{what}: {id} {p}"));
        a.is_ok()
    }

    fn check(&mut self, what: &str) {
        self.steps += 1;
        let (a, b) = (&self.cached, &self.full);
        assert_eq!(a.solids().len(), b.solids().len(), "{what}: solid count");
        for (x, y) in a.solids().iter().zip(b.solids()) {
            assert_eq!(x.name, y.name, "{what}");
            assert_eq!(x.feature, y.feature, "{what}: {}", x.name);
            // Exact equality: every coordinate, curve, surface and name, bit for bit.
            assert!(x.body == y.body, "{what}: body {} differs", x.name);
            let (va, vb) = (x.body.volume().ok(), y.body.volume().ok());
            assert_eq!(
                va.map(f64::to_bits),
                vb.map(f64::to_bits),
                "{what}: volume of {}",
                x.name
            );
            assert_eq!(
                x.body.summary(),
                y.body.summary(),
                "{what}: faces of {}",
                x.name
            );
            let names = |s: &solids::Solid| {
                s.body
                    .faces()
                    .into_iter()
                    .map(|f| s.body.face_name(f).ok().flatten().map(str::to_owned))
                    .collect::<Vec<_>>()
            };
            assert_eq!(names(x), names(y), "{what}: face names of {}", x.name);
            assert_eq!(x.faces, y.faces, "{what}: mesh faces of {}", x.name);
            assert_eq!(x.edges, y.edges, "{what}: mesh edges of {}", x.name);
        }
        // Everything else the rebuild works out: errors, constructs, sheets, placements.
        assert_eq!(
            format!("{:?}", a.built),
            format!("{:?}", b.built),
            "{what}: built"
        );
        assert!(
            a.doc == b.doc,
            "{what}: the designs differ (followed sketches)"
        );
        assert_eq!(
            format!("{:?}", a.scene.bodies),
            format!("{:?}", b.scene.bodies),
            "{what}: scene"
        );
        assert_eq!(
            format!("{:?}", a.cam),
            format!("{:?}", b.cam),
            "{what}: toolpaths"
        );
    }
}

/// A fixed-seed generator (64-bit LCG, Knuth's MMIX constants), so failures repeat.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// Runs `script` on both twins, then undoes every step and redoes them (comparing after
/// each), then makes `edits` random edits: parameter changes, suppressing features,
/// rolling the timeline, undo and redo.
fn exercise(file: &Path, edits: usize, seed: u64) -> Twins {
    let name = file.file_stem().unwrap().to_string_lossy().into_owned();
    let out = std::env::temp_dir().join(format!("chamfera-perf-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let text = std::fs::read_to_string(file).unwrap();
    let steps = fix_paths(&serde_json::from_str(&text).unwrap(), &out);
    let steps = steps.as_array().unwrap();
    let mut t = Twins::new();
    let a = script::run_steps(&mut t.cached, steps).map_err(|e| e.to_string());
    let b = script::run_steps(&mut t.full, steps).map_err(|e| e.to_string());
    assert_eq!(a.is_ok(), b.is_ok(), "{name}: script");
    t.check(&format!("{name}: script"));
    let mut undone = 0;
    while t.cached.can_undo() {
        assert!(t.run("edit.undo", &json!({}), &format!("{name}: undo {undone}")));
        undone += 1;
    }
    for k in 0..undone {
        assert!(t.run("edit.redo", &json!({}), &format!("{name}: redo {k}")));
    }
    let originals: Vec<(String, String)> = t
        .cached
        .doc
        .parameters
        .iter()
        .map(|(n, p)| {
            let e = p.expression.clone().unwrap_or_else(|| p.value.to_string());
            (n.clone(), e)
        })
        .collect();
    let mut rng = Lcg(seed);
    for k in 0..edits {
        let what = format!("{name}: edit {k}");
        let features: Vec<u32> = t.cached.doc.features.iter().map(|f| f.id).collect();
        let items = t.cached.doc.timeline_order().len();
        let configs: Vec<String> = t
            .cached
            .doc
            .configurations
            .list
            .iter()
            .map(|c| c.name.clone())
            .collect();
        match rng.below(8) {
            6 => {
                // Another unit changes how plain numbers in expressions are read.
                let unit = ["mm", "in", "cm"][rng.below(3)];
                t.run("design.units", &json!({ "length": unit }), &what);
            }
            7 => {
                let config = (!configs.is_empty() && rng.below(3) > 0)
                    .then(|| configs[rng.below(configs.len())].clone());
                t.run("config.activate", &json!({ "config": config }), &what);
            }
            0 | 1 if !originals.is_empty() => {
                let (n, e) = &originals[rng.below(originals.len())];
                let f = [0.9, 0.95, 1.0, 1.05, 1.1][rng.below(5)];
                let expression = format!("{f} * ({e})");
                t.run(
                    "param.set",
                    &json!({ "name": n, "expression": expression }),
                    &what,
                );
            }
            2 if !features.is_empty() => {
                let id = features[rng.below(features.len())];
                let on = rng.below(2) == 0;
                t.run(
                    "feature.edit",
                    &json!({ "feature": id, "suppressed": on }),
                    &what,
                );
            }
            3 if items > 0 => {
                let at = rng.below(items + 1);
                t.run("timeline.roll_to", &json!({ "position": at }), &what);
                if rng.below(2) == 0 {
                    t.run("timeline.roll_to_end", &json!({}), &what);
                }
            }
            4 => {
                t.run("edit.undo", &json!({}), &what);
            }
            _ => {
                t.run("edit.redo", &json!({}), &what);
            }
        }
    }
    t
}

fn scripts(dir: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(repo().join(dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        // Scripts that save or export write files beside the working directory; they are
        // covered by their own tests in a scratch folder.
        .filter(|p| {
            std::fs::read_to_string(p)
                .is_ok_and(|s| !s.contains("\"design.save\"") && !s.contains("\"path\""))
        })
        .collect();
    files.sort();
    files
}

#[test]
fn incremental_rebuilds_match_full_rebuilds_for_every_example() {
    let files = scripts("examples");
    assert!(files.len() >= 20, "{files:?}");
    for (k, f) in files.iter().enumerate() {
        let t = exercise(f, 12, 0x5EED_0000 + k as u64);
        assert!(t.steps > 12, "{f:?}");
    }
}

#[test]
fn incremental_rebuilds_match_full_rebuilds_for_the_qa_parts() {
    for (k, f) in scripts("examples/qa").iter().enumerate() {
        // Part 2 (the flanged pipe) is the slowest part; fewer random edits keep the test
        // short, its undo and redo still visit every state.
        let edits = if f.ends_with("02-flanged-pipe.json") {
            4
        } else {
            12
        };
        exercise(f, edits, 0x0A11_0000 + k as u64);
    }
}

#[test]
fn undo_and_an_edit_to_the_last_feature_reuse_the_checkpoints() {
    let mut t = exercise(&repo().join("examples/qa/01-enclosure.json"), 0, 1);
    let order = t.cached.doc.timeline_order();
    let items = order.len();
    let last = *order
        .iter()
        .rev()
        .find(|id| t.cached.doc.feature(**id).is_ok())
        .unwrap();
    let position = order.iter().position(|id| *id == last).unwrap();
    // A different spacing builds a different pattern: only the last item changed, so
    // everything before it is reused.
    let spacing = json!({"feature": last, "spacing": "width - 2 * inset - 4 mm"});
    assert!(t.run("feature.edit", &spacing, "spacing"));
    let stats = t.cached.rebuild_cache_usage().stats;
    assert_eq!(stats.last_reused, position, "{stats:?}");
    assert_eq!(stats.last_built, items - position, "{stats:?}");
    assert!(t.run("edit.undo", &json!({}), "undo"));
    // The design before the edit was built before: nothing is built again, and no solid
    // is tessellated again.
    let misses = t.cached.rebuild_cache_usage().stats.mesh_misses;
    assert!(t.run("edit.redo", &json!({}), "redo"));
    assert!(t.run("edit.undo", &json!({}), "undo again"));
    let after = t.cached.rebuild_cache_usage().stats;
    assert_eq!(after.last_built, 0, "{after:?}");
    assert_eq!(after.last_reused, items, "{after:?}");
    assert_eq!(after.mesh_misses, misses, "{after:?}");
}

/// A hostile design for the caches: 40 plates side by side, each on its own sketch, and a
/// chain of parameters where each refers to the one before twice.
fn many_features(t: &mut Twins) {
    t.run(
        "param.set",
        &json!({"name": "p0", "expression": "2 mm"}),
        "p0",
    );
    for i in 1..60 {
        let e = format!("p{} + p{} - p{}", i - 1, i - 1, i - 1);
        t.run(
            "param.set",
            &json!({"name": format!("p{i}"), "expression": e}),
            "chain",
        );
    }
    for i in 0..40 {
        let x = f64::from(i) * 12.0;
        t.run("sketch.create", &json!({"plane": "xy"}), "sketch");
        t.run(
            "sketch.rectangle",
            &json!({"x1": x, "y1": 0, "x2": x + 10.0, "y2": 10}),
            "rect",
        );
        t.run("sketch.finish", &json!({}), "finish");
        let sketch = t.cached.doc.sketches.last().unwrap().id;
        t.run(
            "solid.extrude",
            &json!({"sketch": sketch, "profiles": [{"x": x + 5.0, "y": 5}], "distance": "p59"}),
            "extrude",
        );
    }
}

#[test]
fn the_caches_stay_within_their_limits_on_a_hostile_design() {
    let mut t = Twins::new();
    // Limits far below what the design needs: the caches must forget, not grow.
    let limit = 64 * 1024;
    t.cached.set_rebuild_cache_limits(limit, limit);
    many_features(&mut t);
    let check = |s: &Session| {
        let u = s.rebuild_cache_usage();
        assert!(
            u.checkpoint_bytes <= limit && u.mesh_bytes <= limit,
            "{u:?}"
        );
        assert!(u.checkpoints <= perf::MAX_CACHE_ENTRIES && u.meshes <= perf::MAX_CACHE_ENTRIES);
    };
    check(&t.cached);
    assert_eq!(t.cached.solids().len(), 40);
    // Each plate is 10 x 10 x p59, p59 = 2 mm (each step adds and takes p(i-1) once).
    let v = t.cached.solids()[39].body.volume().unwrap();
    assert!((v - 200.0).abs() < 1e-9, "{v}");
    for k in 0..10 {
        t.run(
            "param.set",
            &json!({"name": "p0", "expression": format!("{} mm", 2 + k % 3)}),
            "p0",
        );
        check(&t.cached);
        t.run("edit.undo", &json!({}), "undo");
        check(&t.cached);
    }
    // Nothing fits at all: every rebuild builds everything, and still agrees.
    t.cached.set_rebuild_cache_limits(0, 0);
    let u = t.cached.rebuild_cache_usage();
    assert_eq!(
        (u.checkpoints, u.meshes, u.checkpoint_bytes, u.mesh_bytes),
        (0, 0, 0, 0)
    );
    t.run(
        "param.set",
        &json!({"name": "p0", "expression": "3 mm"}),
        "no room",
    );
    let u = t.cached.rebuild_cache_usage();
    assert_eq!((u.checkpoints, u.meshes), (0, 0), "{u:?}");
}

#[test]
fn default_cache_limits_are_a_few_hundred_megabytes() {
    let u = Session::new().rebuild_cache_usage();
    assert_eq!(u.checkpoint_limit, 256 * 1024 * 1024);
    assert_eq!(u.mesh_limit, 128 * 1024 * 1024);
}

#[test]
fn the_timing_report_splits_each_feature_and_checks_its_parameters() {
    let mut t = exercise(&repo().join("examples/qa/01-enclosure.json"), 0, 2);
    // Both twins time (each timed run rebuilds, giving the solids new scene ids).
    t.full
        .execute("debug.timings", &json!({"runs": 2}))
        .unwrap();
    let s = &mut t.cached;
    let r = s.execute("debug.timings", &json!({"runs": 2})).unwrap();
    assert_eq!(r["runs"], 2);
    let features = r["features"].as_array().unwrap();
    // One entry per timeline item: the enclosure has 10 (sketches and features).
    assert_eq!(features.len(), s.doc.timeline_order().len());
    let kinds: Vec<&str> = features
        .iter()
        .map(|f| f["kind"].as_str().unwrap())
        .collect();
    for k in ["sketch", "extrude", "blend", "shell", "hole", "pattern"] {
        assert!(kinds.contains(&k), "{kinds:?}");
    }
    for f in features {
        let part = |k: &str| f[k].as_f64().unwrap();
        for k in ["kernel_ms", "booleans_ms", "naming_ms", "tessellation_ms"] {
            assert!(part(k) >= 0.0, "{f}");
        }
        let sum =
            part("kernel_ms") + part("booleans_ms") + part("naming_ms") + part("tessellation_ms");
        assert!((sum - part("total_ms")).abs() < 1e-6 * sum.max(1.0), "{f}");
    }
    // The cut and the hole are booleans; the solids are tessellated.
    assert!(r["totals_ms"]["booleans"].as_f64().unwrap() > 0.0, "{r}");
    assert!(
        r["totals_ms"]["tessellation"].as_f64().unwrap() > 0.0,
        "{r}"
    );
    for bad in [
        json!({"runs": 0}),
        json!({"runs": 51}),
        json!({"runs": 2.5}),
        json!({"runs": "3"}),
        json!({"cached": 1}),
    ] {
        assert!(s.execute("debug.timings", &bad).is_err(), "{bad}");
    }
    // Timing leaves the solids as they were.
    t.check("after timing");
    let u = t.cached.execute("debug.cache", &json!({})).unwrap();
    assert!(u["checkpoints"].as_u64().unwrap() > 0, "{u}");
}
