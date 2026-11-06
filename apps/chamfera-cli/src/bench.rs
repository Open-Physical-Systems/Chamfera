//! `chamfera-cli bench`: how long rebuilds and edits take on the example designs.
//!
//! For each script it reports the full rebuild split into kernel, booleans, naming and
//! tessellation (`debug.timings`), then times the same edits with the rebuild cache off
//! (every change rebuilds the whole timeline) and on: undoing and redoing the script's
//! last step, and suppressing and restoring the last feature. Run it from the repository
//! root in a release build: `cargo run --release -p chamfera-cli -- bench`. With `--cam`
//! it times edits of the designs with machining operations instead: toolpaths made again
//! on the spot, the time the UI waits with background generation, the worker's time, and
//! undo and redo from the toolpath cache.

use chamfera_engine::Session;
use serde_json::{Value, json};
use std::time::Instant;

/// Each design's numbers, in milliseconds.
struct Row {
    name: String,
    items: usize,
    script_ms: [f64; 2],
    rebuild_ms: f64,
    kernel_ms: f64,
    booleans_ms: f64,
    naming_ms: f64,
    tessellation_ms: f64,
    other_ms: f64,
    /// Cache off, on: undo, redo, suppress last, restore last.
    edits: [[f64; 4]; 2],
    slowest: String,
}

fn default_scripts() -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for dir in ["examples", "examples/qa"] {
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .map_err(|e| format!("{dir}: {e} (run bench from the repository root)"))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        files.sort();
        out.extend(files);
    }
    Ok(out)
}

fn exec(s: &mut Session, id: &str, p: Value) -> Result<Value, String> {
    s.execute(id, &p).map_err(|e| format!("{id}: {e}"))
}

/// Milliseconds `f` takes.
fn time(f: impl FnOnce() -> Result<(), String>) -> Result<f64, String> {
    let t = Instant::now();
    f()?;
    Ok(t.elapsed().as_secs_f64() * 1000.0)
}

fn run_steps(s: &mut Session, steps: &[Value]) -> Result<(), String> {
    chamfera_engine::script::run_steps(s, steps)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Undo, redo, suppress the last feature, restore it: the mean time of each over `runs`
/// rounds.
fn edits(s: &mut Session, runs: usize) -> Result<[f64; 4], String> {
    let last = s
        .doc
        .timeline_order()
        .into_iter()
        .rev()
        .find(|id| s.doc.feature(*id).is_ok());
    let mut sum = [0.0; 4];
    for _ in 0..runs {
        if s.can_undo() {
            sum[0] += time(|| exec(s, "edit.undo", json!({})).map(|_| ()))?;
            sum[1] += time(|| exec(s, "edit.redo", json!({})).map(|_| ()))?;
        }
        if let Some(id) = last {
            let edit = |on: bool| json!({ "feature": id, "suppressed": on });
            sum[2] += time(|| exec(s, "feature.edit", edit(true)).map(|_| ()))?;
            sum[3] += time(|| exec(s, "feature.edit", edit(false)).map(|_| ()))?;
        }
    }
    Ok(sum.map(|x| x / runs as f64))
}

fn bench_one(path: &str, runs: usize) -> Result<Row, String> {
    let steps = crate::load_script(path)?;
    let mut script_ms = [0.0; 2];
    let mut edit_ms = [[0.0; 4]; 2];
    let mut timings = Value::Null;
    for (k, cached) in [false, true].into_iter().enumerate() {
        let mut s = Session::new();
        s.set_rebuild_cache(cached);
        script_ms[k] = time(|| run_steps(&mut s, &steps))?;
        if !cached {
            timings = exec(&mut s, "debug.timings", json!({ "runs": runs }))?;
        } else {
            s.rebuild();
        }
        edit_ms[k] = edits(&mut s, runs)?;
    }
    let f = |path: &[&str]| {
        path.iter()
            .try_fold(&timings, |v, k| v.get(*k))
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    let features = timings
        .get("features")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let slowest = features
        .iter()
        .max_by(|a, b| {
            let t = |v: &Value| v.get("total_ms").and_then(Value::as_f64).unwrap_or(0.0);
            t(a).total_cmp(&t(b))
        })
        .map(|v| {
            format!(
                "{} {:.0}",
                v.get("name").and_then(Value::as_str).unwrap_or("?"),
                v.get("total_ms").and_then(Value::as_f64).unwrap_or(0.0)
            )
        })
        .unwrap_or_default();
    Ok(Row {
        name: std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        items: features.len(),
        script_ms,
        rebuild_ms: f(&["rebuild_ms", "mean"]),
        kernel_ms: f(&["totals_ms", "kernel"]),
        booleans_ms: f(&["totals_ms", "booleans"]),
        naming_ms: f(&["totals_ms", "naming"]),
        tessellation_ms: f(&["totals_ms", "tessellation"]),
        other_ms: f(&["totals_ms", "other"]),
        edits: edit_ms,
        slowest,
    })
}

/// `bench --cam`: for each design with machining operations, an edit to the part (the
/// last feature suppressed, then restored): with every toolpath made again on the spot
/// (the toolpath cache off, as before), the time the UI waits with background generation
/// (the rebuild, the toolpath keys and the job's snapshot; the cache emptied first, so
/// every 3D toolpath misses), the worker's time for that job, and undo and redo with the
/// cache on (every toolpath taken from the cache).
fn bench_cam(path: &str, runs: usize) -> Result<Option<[f64; 5]>, String> {
    let steps = crate::load_script(path)?;
    let mut s = Session::new();
    run_steps(&mut s, &steps)?;
    if s.doc.cam.operations.is_empty() {
        return Ok(None);
    }
    let Some(last) = s
        .doc
        .timeline_order()
        .into_iter()
        .rev()
        .find(|id| s.doc.feature(*id).is_ok())
    else {
        return Ok(None);
    };
    let edit = |on: bool| json!({ "feature": last, "suppressed": on });
    let mut out = [0.0; 5];
    // Before: every toolpath made again after the edit.
    s.set_toolpath_cache(false);
    for _ in 0..runs {
        exec(&mut s, "feature.edit", edit(true))?;
        out[0] += time(|| exec(&mut s, "feature.edit", edit(false)).map(|_| ()))?;
    }
    // Background: what the UI waits for, then the worker's job.
    s.set_toolpath_cache(true);
    s.set_cam_background(true);
    for _ in 0..runs {
        exec(&mut s, "feature.edit", edit(true))?;
        s.ensure_cam_current();
        s.set_toolpath_cache(false);
        s.set_toolpath_cache(true);
        out[1] += time(|| exec(&mut s, "feature.edit", edit(false)).map(|_| ()))?;
        let job = s.take_cam_job();
        out[2] += time(|| {
            if let Some(j) = job {
                let r = j.run();
                s.accept_cam_result(r);
            }
            Ok(())
        })?;
    }
    s.set_cam_background(false);
    // Undo and redo with the cache on, once to fill it.
    exec(&mut s, "edit.undo", json!({}))?;
    exec(&mut s, "edit.redo", json!({}))?;
    for _ in 0..runs {
        out[3] += time(|| exec(&mut s, "edit.undo", json!({})).map(|_| ()))?;
        out[4] += time(|| exec(&mut s, "edit.redo", json!({})).map(|_| ()))?;
    }
    Ok(Some(out.map(|x| x / runs as f64)))
}

pub fn bench(args: &[String]) -> Result<(), crate::Stop> {
    let mut runs = 3usize;
    let mut scripts = Vec::new();
    let mut cam = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--runs" => {
                runs = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|n| (1..=50).contains(n))
                    .ok_or("--runs needs a whole number from 1 to 50")?;
            }
            "--cam" => cam = true,
            other if other.starts_with("--") => {
                return Err(format!("unknown option {other:?}").into());
            }
            path => scripts.push(path.to_owned()),
        }
    }
    if scripts.is_empty() {
        scripts = default_scripts()?;
    }
    // The QA scripts save into target/qa/.
    std::fs::create_dir_all("target/qa").map_err(|e| format!("target/qa: {e}"))?;
    if cam {
        crate::out(
            "| Design | Edit, every toolpath made again | Edit, UI waits (background) | Worker | Undo (cached) | Redo (cached) |",
        )?;
        crate::out("|---|---|---|---|---|---|")?;
        for path in &scripts {
            match bench_cam(path, runs) {
                Ok(Some(r)) => crate::out(format_args!(
                    "| {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
                    std::path::Path::new(path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    r[0],
                    r[1],
                    r[2],
                    r[3],
                    r[4]
                ))?,
                Ok(None) => {}
                Err(e) => crate::out(format_args!("| {path} | failed: {e} |"))?,
            }
        }
        return Ok(());
    }
    crate::out(
        "| Design | Items | Script off / on | Full rebuild | Kernel | Booleans | Naming | Tess. | Other | Undo off / on | Redo off / on | Edit last off / on | Slowest item |",
    )?;
    crate::out("|---|---|---|---|---|---|---|---|---|---|---|---|---|")?;
    for path in &scripts {
        match bench_one(path, runs) {
            Ok(r) => crate::out(format_args!(
                "| {} | {} | {:.0} / {:.0} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} / {:.1} | {:.1} / {:.1} | {:.1} / {:.1} | {} |",
                r.name,
                r.items,
                r.script_ms[0],
                r.script_ms[1],
                r.rebuild_ms,
                r.kernel_ms,
                r.booleans_ms,
                r.naming_ms,
                r.tessellation_ms,
                r.other_ms,
                r.edits[0][0],
                r.edits[1][0],
                r.edits[0][1],
                r.edits[1][1],
                r.edits[0][3],
                r.edits[1][3],
                r.slowest,
            ))?,
            Err(e) => crate::out(format_args!("| {path} | failed: {e} |"))?,
        }
    }
    Ok(())
}
