//! STEP (ISO 10303) export of B-rep bodies, from the public standard: the exchange
//! structure of ISO 10303-21 (Part 21), the geometric and topological resources of
//! ISO 10303-42 and the product and representation entities shared by AP203/AP214/AP242,
//! following the CAx-IF recommended practices for the product structure and units.
//!
//! # Schema
//! Files declare AP214, `FILE_SCHEMA(('AUTOMOTIVE_DESIGN'))`. Every entity we write
//! (product structure, units, advanced B-rep) is in both AP214 and AP242 with the same
//! attributes, and AP214 is read by the widest range of importers, old and new. Moving to
//! AP242 later is a change of the schema name and the application context, not of the
//! geometry.
//!
//! # What is written
//! One product (named by [`StepOptions::product_name`]) whose shape is an
//! `ADVANCED_BREP_SHAPE_REPRESENTATION` in millimetres and radians, with a
//! `LINEAR_TOLERANCE` uncertainty of [`chamfera_geom::LINEAR_TOLERANCE`]. Each shell of
//! each body becomes a `MANIFOLD_SOLID_BREP` (named after the body) over a `CLOSED_SHELL`
//! of `ADVANCED_FACE`s. Topology maps one to one:
//!
//! | Kernel | STEP |
//! |---|---|
//! | shell | `CLOSED_SHELL` in a `MANIFOLD_SOLID_BREP` |
//! | face (surface, `same_sense`) | `ADVANCED_FACE(name, bounds, surface, same_sense)` |
//! | outer loop / inner loop | `FACE_OUTER_BOUND` / `FACE_BOUND` (orientation `.T.`) over an `EDGE_LOOP` |
//! | half-edge | `ORIENTED_EDGE`, `.T.` when it runs along its edge (the edge's first half-edge) |
//! | edge | `EDGE_CURVE(name, start, end, curve, .T.)`, written once, used by both faces |
//! | vertex | `VERTEX_POINT` over a `CARTESIAN_POINT`, written once |
//!
//! Our loops already follow the Part 42 convention (the face lies to the left of its
//! bounds, seen against the face's outward normal), so every bound has orientation `.T.`.
//! An edge's first half-edge runs from `c(t0)` to `c(t1)` with `t0 < t1`, i.e. in the
//! curve's parameter direction, and our curves are parameterised as their STEP
//! counterparts are, so `EDGE_CURVE.same_sense` is always `.T.`.
//!
//! Curves: segments and lines become `LINE` with a `VECTOR`; circles and circular arcs
//! `CIRCLE`; ellipses and elliptical arcs `ELLIPSE` (`semi_axis_1` along the frame's x).
//! An edge is bounded by its vertices, so arcs need no `TRIMMED_CURVE`.
//!
//! Surfaces: `PLANE`, `CYLINDRICAL_SURFACE`, `CONICAL_SURFACE`, `SPHERICAL_SURFACE`,
//! `TOROIDAL_SURFACE` and, for spindle and lemon tori, `DEGENERATE_TOROIDAL_SURFACE`,
//! each on the surface's frame (a cone narrowing toward +z on its frame turned over, as
//! Part 42's semi-angle is positive). `ADVANCED_FACE.same_sense` is the kernel's flag,
//! flipped where Part 42's normal (`σ_u × σ_v` of the entity) is against the kernel's:
//! only on the lemon part of a degenerate torus. See `write::StepSurface` for the
//! mapping and its derivation.
//!
//! # Reading
//! [`read`] parses Part 21 text into entity instances and [`read::summarize`] rebuilds
//! the B-rep counts and vertex coordinates. It exists to check our own output; full STEP
//! import (mapping foreign files into kernel bodies) is a later task.

mod format;
pub mod read;
mod write;

#[cfg(test)]
mod tests;

pub use read::{
    BrepSummary, Exchange, Instance, Param, Record, SolidSummary, SurfaceSummary, summarize,
};
pub use write::write;

use crate::IoError;
use chamfera_kernel::Body;

/// Most entity instances one STEP file may hold, written or read (hostile-input limit).
pub const MAX_STEP_ENTITIES: usize = 50_000_000;
/// Largest STEP file [`read::parse`] accepts (bytes).
pub const MAX_STEP_BYTES: usize = 1024 * 1024 * 1024;
/// Longest text (product, author, organisation, file name, body name) the writer accepts,
/// in bytes.
pub const MAX_STEP_TEXT: usize = 4096;
/// The schema every file declares.
pub const SCHEMA: &str = "AUTOMOTIVE_DESIGN";

/// Header and product data for a STEP file.
#[derive(Debug, Clone, PartialEq)]
pub struct StepOptions {
    /// `FILE_NAME.name`, usually the file's base name.
    pub file_name: String,
    /// The product's id and name.
    pub product_name: String,
    pub author: String,
    pub organization: String,
    /// `FILE_NAME.time_stamp`, ISO 8601 (`2026-11-06T12:00:00`). `None` writes the Unix
    /// epoch from [`write`] (which has no clock, so it builds for wasm) and the current
    /// time from [`write_file`].
    pub timestamp: Option<String>,
    /// Refuse to write more entity instances than this.
    pub max_entities: usize,
}

impl Default for StepOptions {
    fn default() -> Self {
        StepOptions {
            file_name: String::new(),
            product_name: "Part".into(),
            author: String::new(),
            organization: String::new(),
            timestamp: None,
            max_entities: MAX_STEP_ENTITIES,
        }
    }
}

/// Writes `bodies` to a STEP file at `path`. An empty `options.file_name` becomes the
/// path's file name and a missing timestamp the current time (UTC).
pub fn write_file(
    path: &std::path::Path,
    bodies: &[(&str, &Body)],
    options: &StepOptions,
) -> Result<(), IoError> {
    let mut options = options.clone();
    if options.file_name.is_empty() {
        options.file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    if options.timestamp.is_none() {
        options.timestamp = Some(now_iso8601());
    }
    let text = write(bodies, &options)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// The current UTC time as `YYYY-MM-DDThh:mm:ss`.
fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso8601(secs)
}

/// `secs` after the Unix epoch as `YYYY-MM-DDThh:mm:ss` (UTC), by the proleptic
/// Gregorian calendar: days to (year, month, day) with the 400-year era decomposition
/// (March-based years, so the leap day ends a year).
fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468; // days since 0000-03-01
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // March = 0
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

#[cfg(test)]
mod clock_tests {
    use super::iso8601;

    #[test]
    fn timestamps_follow_the_gregorian_calendar() {
        // Hand-checked dates: the epoch; 2000-02-29 (a leap day in a century year
        // divisible by 400) is day 11016 since the epoch; 2026-11-06 is day 20763
        // (56 years with 14 leap days to 2026-01-01 = 20454, plus 309 days).
        assert_eq!(iso8601(0), "1970-01-01T00:00:00");
        assert_eq!(iso8601(11_016 * 86_400 + 3_661), "2000-02-29T01:01:01");
        assert_eq!(iso8601(20_763 * 86_400 + 86_399), "2026-11-06T23:59:59");
    }
}
