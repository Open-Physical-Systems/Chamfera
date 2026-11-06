//! `debug.*`: looking at how the session itself performs.

use super::CommandSpec;
use crate::perf::FeatureTiming;
use crate::{CmdResult, CommandError, Session, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "debug.cache",
        label: "Rebuild Cache Usage",
        params: "(none)",
        mutates: false,
        run: cache,
    },
    CommandSpec {
        id: "debug.timings",
        label: "Rebuild Timings",
        params: "runs: number (optional) - how many rebuilds to time, 1 to 50 (default 3)\n\
                 cached: bool (optional) - keep the rebuild cache (default false: every \
                 rebuild builds the whole timeline)",
        mutates: false,
        run: timings,
    },
];

/// Most rebuilds one `debug.timings` call times.
const MAX_RUNS: f64 = 50.0;

fn cache(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    serde_json::to_value(s.rebuild_cache_usage())
        .map_err(|e| CommandError::Unavailable(e.to_string()))
}

fn timings(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let runs = params::opt_f64(p, "runs")?.unwrap_or(3.0);
    if runs.fract() != 0.0 || !(1.0..=MAX_RUNS).contains(&runs) {
        return Err(CommandError::BadParams(format!(
            "`runs` must be a whole number from 1 to {MAX_RUNS}"
        )));
    }
    let cached = match p.get("cached") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "cached")?,
    };
    if !crate::perf::has_clock() {
        return Err(CommandError::Unavailable(
            "this build has no clock to time rebuilds with".into(),
        ));
    }
    let runs = runs as usize;
    let mut walls = Vec::with_capacity(runs);
    // Per item: the sum of each run's times.
    let mut sums: BTreeMap<u32, FeatureTiming> = BTreeMap::new();
    let mut order: Vec<u32> = Vec::new();
    crate::perf::set_timing(true);
    for _ in 0..runs {
        if !cached {
            s.caches.clear();
        }
        s.timings.clear();
        let clock = crate::perf::Stopwatch::start();
        s.rebuild();
        walls.push(clock.ms());
        for t in s.timings.drain(..) {
            let e = sums.entry(t.id).or_insert_with(|| {
                order.push(t.id);
                FeatureTiming {
                    id: t.id,
                    name: t.name.clone(),
                    kind: t.kind.clone(),
                    ..FeatureTiming::default()
                }
            });
            e.kernel_ms += t.kernel_ms;
            e.booleans_ms += t.booleans_ms;
            e.naming_ms += t.naming_ms;
            e.tessellation_ms += t.tessellation_ms;
            e.total_ms += t.total_ms;
        }
    }
    crate::perf::set_timing(false);
    let n = runs as f64;
    let features: Vec<FeatureTiming> = order
        .iter()
        .filter_map(|id| sums.get(id))
        .map(|t| FeatureTiming {
            kernel_ms: t.kernel_ms / n,
            booleans_ms: t.booleans_ms / n,
            naming_ms: t.naming_ms / n,
            tessellation_ms: t.tessellation_ms / n,
            total_ms: t.total_ms / n,
            ..t.clone()
        })
        .collect();
    let sum = |f: fn(&FeatureTiming) -> f64| features.iter().map(f).sum::<f64>();
    let mean = walls.iter().sum::<f64>() / n;
    let min = walls.iter().copied().fold(f64::INFINITY, f64::min);
    Ok(json!({
        "runs": runs,
        "cached": cached,
        "rebuild_ms": { "mean": mean, "min": min },
        "totals_ms": {
            "kernel": sum(|t| t.kernel_ms),
            "booleans": sum(|t| t.booleans_ms),
            "naming": sum(|t| t.naming_ms),
            "tessellation": sum(|t| t.tessellation_ms),
            // Placing components, the scene and toolpaths: the rest of a rebuild.
            "other": (mean - sum(|t| t.total_ms)).max(0.0),
        },
        "features": features,
    }))
}
