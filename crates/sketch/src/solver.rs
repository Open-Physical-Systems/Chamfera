//! The constraint solver: Levenberg-Marquardt on the residual equations, degree-of-freedom
//! analysis, and dragging.
//!
//! **Decomposition.** Unknowns that share no equation cannot influence each other, so the
//! unknowns are split into connected groups (union-find over the equations) and each group
//! is solved on its own. A horizontal line's `y` coordinates, for example, form a group
//! separate from its `x` coordinates.
//!
//! **Solving.** Each group is solved by damped least squares (Levenberg-Marquardt, with the
//! damping update of Madsen, Nielsen and Tingleff, *Methods for Non-Linear Least Squares
//! Problems*, 2004): the step solves `(JᵀJ + μI) δ = -Jᵀr` by Cholesky. The damping is a
//! multiple of the identity in millimetres, so every step lies in the row space of `J`: the
//! smallest move that reduces the residual. Starting from the current geometry the solver
//! walks to the nearest solution. A step that would turn a measured direction by a right
//! angle or more (a line's length passing through zero to the mirror image) or make a
//! radius non-positive is rejected and retried shorter, so a sketch does not flip.
//! Iterations are bounded ([`MAX_ITERATIONS`]); a group has converged when every residual is
//! at most `LINEAR_TOLERANCE` (it aims for [`SOLVE_TARGET`]).
//!
//! **Failure.** A solve works on a copy of the unknowns and writes them back only when every
//! group converged and the result passes [`Sketch::validate`], so a failed solve leaves the
//! sketch exactly as it was. When a group fails, rows of the Jacobian that depend on earlier
//! rows (constraints in id order) are dropped and the rest solved; dropped constraints that
//! are then satisfied are redundant, the others conflicting ([`SolveError::Conflicting`]).
//! Without dependent rows the group has no real solution (for example a triangle with sides
//! 3, 4 and 10) and [`SolveError::NoSolution`] lists the unsatisfied constraints.
//!
//! **Degrees of freedom.** The rank of the Jacobian at the solution, from a Householder QR
//! of `Jᵀ` built row by row (`linalg.rs`; a row counts as dependent when less than 1e-8 of
//! its length lies outside the span of the rows before it), gives the remaining degrees of
//! freedom: unknowns minus rank. An unknown is determined when its coordinate axis lies in
//! the row space of `J` (within [`DETERMINED_TOLERANCE`]): no motion that keeps the
//! constraints can change it. This is the per-point status the UI colours.
//!
//! **Dragging.** [`Sketch::drag`] moves a point toward a target while keeping every
//! constraint. Each step is the minimum-norm move that keeps the constraints to first order
//! and brings the point as close to the target as the null space of `J` allows; it is
//! projected back onto the constraints (Levenberg-Marquardt) and accepted only if it made
//! progress, otherwise halved. Only the groups coupled to the point move. A point that
//! cannot reach the target stops at the closest place it can reach; a fully determined
//! point does not move.

use crate::entity::{ConstraintId, CurveId, PointId};
use crate::equations::{Equation, Layout, compile};
use crate::linalg::{RowSpace, cholesky_solve, dot, norm};
use crate::sketch::{Sketch, SketchError};
use chamfera_geom::{LINEAR_TOLERANCE, Point2, is_valid_coordinate};
use std::collections::{BTreeMap, BTreeSet};

/// Most Levenberg-Marquardt iterations (accepted or rejected steps) per group.
pub const MAX_ITERATIONS: usize = 100;
/// Most steps of a drag.
pub const MAX_DRAG_STEPS: usize = 100;
/// Residual (mm) the solver aims for; it accepts anything up to `LINEAR_TOLERANCE`.
pub const SOLVE_TARGET: f64 = LINEAR_TOLERANCE * 1e-3;
/// Largest group of coupled unknowns solved with dense linear algebra. Larger groups are
/// refused ([`SolveError::TooLarge`]) so a solve always finishes in bounded time; a sparse
/// solver can lift this later.
pub const MAX_GROUP_UNKNOWNS: usize = 600;
/// Largest number of equations in one group of coupled unknowns.
pub const MAX_GROUP_EQUATIONS: usize = 2400;
/// An unknown counts as determined when the part of its axis outside the Jacobian's row
/// space is shorter than this (the axis has length 1).
pub const DETERMINED_TOLERANCE: f64 = 1e-6;

/// Damping beyond which a step is too small to matter.
const MAX_DAMPING: f64 = 1e30;
/// A step shorter than this fraction of the unknowns' size has stalled.
const STEP_TOLERANCE: f64 = 1e-15;

/// How constrained a sketch is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintStatus {
    /// Some geometry can still move.
    UnderConstrained,
    /// Nothing can move and no constraint is redundant.
    FullyConstrained,
    /// Some constraints are redundant (implied by others) or conflicting.
    OverConstrained,
}

/// Degrees of freedom and what is determined, at the sketch's current geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct DofReport {
    /// Unknowns minus the rank of the constraint Jacobian.
    pub dof: usize,
    pub status: ConstraintStatus,
    /// Constraints implied by constraints with smaller ids (all of them satisfied).
    pub redundant: Vec<ConstraintId>,
    /// Points that no constraint-preserving motion can move.
    pub determined_points: BTreeSet<PointId>,
    /// Curves whose points (and radius) are all determined.
    pub determined_curves: BTreeSet<CurveId>,
}

impl DofReport {
    pub fn is_point_determined(&self, id: PointId) -> bool {
        self.determined_points.contains(&id)
    }

    pub fn is_curve_determined(&self, id: CurveId) -> bool {
        self.determined_curves.contains(&id)
    }
}

/// The result of a successful solve.
#[derive(Debug, Clone, PartialEq)]
pub struct SolveReport {
    /// Iterations used by the slowest group.
    pub iterations: usize,
    /// Largest residual left (mm).
    pub max_residual: f64,
    pub dof: DofReport,
}

/// The result of a successful drag.
#[derive(Debug, Clone, PartialEq)]
pub struct DragReport {
    pub steps: usize,
    /// Where the dragged point ended up.
    pub reached: Point2,
    /// How far that is from the target (0 when the target was reachable).
    pub distance_to_target: f64,
}

/// Why a solve or drag failed. The sketch is unchanged.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SolveError {
    #[error("the sketch is invalid: {0}")]
    Invalid(#[from] SketchError),
    #[error(
        "constraints {conflicting:?} conflict with the others (redundant: {redundant:?}); \
         remove or change one of them"
    )]
    Conflicting {
        conflicting: Vec<ConstraintId>,
        redundant: Vec<ConstraintId>,
    },
    #[error("no geometry satisfies constraints {unsatisfied:?} (largest error {max_residual} mm)")]
    NoSolution {
        unsatisfied: Vec<ConstraintId>,
        max_residual: f64,
    },
    #[error(
        "a group of {unknowns} coupled unknowns and {equations} equations is larger than the \
         solver's limit of {MAX_GROUP_UNKNOWNS} unknowns and {MAX_GROUP_EQUATIONS} equations"
    )]
    TooLarge { unknowns: usize, equations: usize },
    #[error("the solution is degenerate: {0}")]
    Degenerate(String),
    #[error("the solver met a non-finite number")]
    NonFinite,
}

/// The compiled sketch: unknowns, equations and the groups they split into.
struct System {
    layout: Layout,
    eqs: Vec<Equation>,
    groups: Vec<Group>,
    /// Each unknown's column inside its group.
    col: Vec<usize>,
}

struct Group {
    /// Unknowns (indices into `x`), ascending.
    vars: Vec<usize>,
    /// Equations (indices into `eqs`), in compile order.
    eqs: Vec<usize>,
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while let Some(&p) = parent.get(i) {
        if p == i {
            break;
        }
        let gp = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = gp;
        }
        i = gp;
    }
    i
}

impl System {
    fn build(sketch: &Sketch) -> Result<System, SolveError> {
        let layout = Layout::new(sketch);
        let eqs = compile(sketch, &layout)?;
        let n = layout.x.len();
        let mut parent: Vec<usize> = (0..n).collect();
        let mut vars = Vec::new();
        let mut eq_root = Vec::with_capacity(eqs.len());
        for e in &eqs {
            vars.clear();
            e.residual.vars(&mut vars);
            if vars.iter().any(|&v| v >= n) {
                return Err(SolveError::NonFinite);
            }
            let first = vars.first().copied();
            if let Some(f) = first {
                for &v in vars.iter().skip(1) {
                    // Roots are looked up afresh each time: a merge can change f's root.
                    let (rf, rv) = (find(&mut parent, f), find(&mut parent, v));
                    if rv != rf
                        && let Some(slot) = parent.get_mut(rf.max(rv))
                    {
                        *slot = rf.min(rv);
                    }
                }
            }
            eq_root.push(first);
        }
        let mut group_of_root: BTreeMap<usize, usize> = BTreeMap::new();
        let mut groups: Vec<Group> = Vec::new();
        let mut col = vec![usize::MAX; n];
        for v in 0..n {
            let r = find(&mut parent, v);
            let g = *group_of_root.entry(r).or_insert_with(|| {
                groups.push(Group {
                    vars: Vec::new(),
                    eqs: Vec::new(),
                });
                groups.len() - 1
            });
            if let Some(group) = groups.get_mut(g) {
                if let Some(slot) = col.get_mut(v) {
                    *slot = group.vars.len();
                }
                group.vars.push(v);
            }
        }
        for (k, root) in eq_root.iter().enumerate() {
            let Some(first) = root else { continue };
            let r = find(&mut parent, *first);
            if let Some(group) = group_of_root.get(&r).and_then(|g| groups.get_mut(*g)) {
                group.eqs.push(k);
            }
        }
        for g in &groups {
            if g.vars.len() > MAX_GROUP_UNKNOWNS || g.eqs.len() > MAX_GROUP_EQUATIONS {
                return Err(SolveError::TooLarge {
                    unknowns: g.vars.len(),
                    equations: g.eqs.len(),
                });
            }
        }
        Ok(System {
            layout,
            eqs,
            groups,
            col,
        })
    }

    fn equations(&self, which: &[usize]) -> Vec<&Equation> {
        which.iter().filter_map(|&k| self.eqs.get(k)).collect()
    }
}

/// A sparse Jacobian row: (group column, derivative) pairs; a column may repeat.
type SparseRow = Vec<(usize, f64)>;

/// Residuals and sparse Jacobian rows (group columns) of `eqs` at `x`.
fn evaluate(
    eqs: &[&Equation],
    col: &[usize],
    x: &[f64],
) -> Result<(Vec<f64>, Vec<SparseRow>), SolveError> {
    let mut r = Vec::with_capacity(eqs.len());
    let mut rows = Vec::with_capacity(eqs.len());
    for e in eqs {
        let mut row = Vec::new();
        let mut bad = false;
        let v = e.residual.eval(x, &mut |i, d| match col.get(i) {
            Some(&c) if c != usize::MAX && d.is_finite() => row.push((c, d)),
            _ => bad = true,
        });
        if bad || !v.is_finite() {
            return Err(SolveError::NonFinite);
        }
        r.push(v);
        rows.push(row);
    }
    Ok((r, rows))
}

fn residuals(eqs: &[&Equation], x: &[f64]) -> Vec<f64> {
    eqs.iter().map(|e| e.residual.value(x)).collect()
}

fn max_abs(v: &[f64]) -> f64 {
    v.iter().fold(0.0_f64, |m, a| {
        if a.is_finite() {
            m.max(a.abs())
        } else {
            f64::INFINITY
        }
    })
}

fn dense_row(row: &[(usize, f64)], n: usize) -> Vec<f64> {
    let mut d = vec![0.0; n];
    for &(c, v) in row {
        if let Some(slot) = d.get_mut(c) {
            *slot += v;
        }
    }
    d
}

/// `JᵀJ` (row-major, full) and `Jᵀr`. Rows are sparse, so this costs a few dozen
/// multiplications per equation.
fn normal_equations(rows: &[SparseRow], r: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut a = vec![0.0; n * n];
    let mut g = vec![0.0; n];
    for (row, ri) in rows.iter().zip(r) {
        for &(i, vi) in row {
            if let Some(gi) = g.get_mut(i) {
                *gi += vi * ri;
            }
            for &(j, vj) in row {
                if let Some(slot) = a.get_mut(i * n + j) {
                    *slot += vi * vj;
                }
            }
        }
    }
    (a, g)
}

struct LmOutcome {
    converged: bool,
    iterations: usize,
    max_residual: f64,
}

/// Levenberg-Marquardt on `eqs` over the unknowns `vars` (columns `col`), updating `x`.
fn levenberg_marquardt(
    eqs: &[&Equation],
    vars: &[usize],
    col: &[usize],
    x: &mut [f64],
) -> Result<LmOutcome, SolveError> {
    let n = vars.len();
    let (mut r, mut rows) = evaluate(eqs, col, x)?;
    let mut max_r = max_abs(&r);
    if max_r <= SOLVE_TARGET || n == 0 {
        return Ok(LmOutcome {
            converged: max_r <= LINEAR_TOLERANCE,
            iterations: 0,
            max_residual: max_r,
        });
    }
    let mut cost = 0.5 * dot(&r, &r);
    let (mut a, mut g) = normal_equations(&rows, &r, n);
    let max_diag = (0..n)
        .filter_map(|i| a.get(i * n + i))
        .fold(0.0_f64, |m, v| m.max(*v));
    let mut mu = if max_diag > 0.0 {
        1e-3 * max_diag
    } else {
        1e-3
    };
    let mut nu = 2.0;
    let mut iterations = 0;
    let mut old = vec![0.0; n];
    let mut before = x.to_vec();
    while iterations < MAX_ITERATIONS {
        iterations += 1;
        let mut m = a.clone();
        for i in 0..n {
            if let Some(d) = m.get_mut(i * n + i) {
                *d += mu;
            }
        }
        let mut delta: Vec<f64> = g.iter().map(|v| -v).collect();
        if !cholesky_solve(&mut m, n, &mut delta) {
            mu *= nu;
            nu *= 2.0;
            if mu > MAX_DAMPING {
                break;
            }
            continue;
        }
        let xnorm = norm(
            &vars
                .iter()
                .map(|&v| x.get(v).copied().unwrap_or(0.0))
                .collect::<Vec<_>>(),
        );
        if norm(&delta) <= STEP_TOLERANCE * (xnorm + 1.0) {
            break;
        }
        for (k, &v) in vars.iter().enumerate() {
            if let (Some(xv), Some(o), Some(d)) = (x.get_mut(v), old.get_mut(k), delta.get(k)) {
                *o = *xv;
                *xv += d;
            }
        }
        let r_new = residuals(eqs, x);
        let new_cost = 0.5 * dot(&r_new, &r_new);
        // Predicted reduction of the linear model: ½ δᵀ(μδ - g).
        let predicted: f64 = 0.5
            * delta
                .iter()
                .zip(&g)
                .map(|(d, gi)| d * (mu * d - gi))
                .sum::<f64>();
        let rho = if predicted > 0.0 {
            (cost - new_cost) / predicted
        } else if new_cost < cost {
            1.0
        } else {
            -1.0
        };
        let flipped = eqs.iter().any(|e| e.residual.flips(&before, x));
        if new_cost.is_finite() && rho > 0.0 && !flipped {
            match evaluate(eqs, col, x) {
                Ok((r2, rows2)) => {
                    r = r2;
                    rows = rows2;
                }
                Err(_) => {
                    restore(vars, &old, x);
                    mu *= nu;
                    nu *= 2.0;
                    continue;
                }
            }
            cost = new_cost;
            max_r = max_abs(&r);
            before.copy_from_slice(x);
            if max_r <= SOLVE_TARGET {
                break;
            }
            (a, g) = normal_equations(&rows, &r, n);
            let t = 2.0 * rho - 1.0;
            mu *= (1.0 - t * t * t).max(1.0 / 3.0);
            nu = 2.0;
        } else {
            restore(vars, &old, x);
            mu *= nu;
            nu *= 2.0;
            if mu > MAX_DAMPING {
                break;
            }
        }
    }
    Ok(LmOutcome {
        converged: max_r <= LINEAR_TOLERANCE,
        iterations,
        max_residual: max_r,
    })
}

fn restore(vars: &[usize], old: &[f64], x: &mut [f64]) {
    for (k, &v) in vars.iter().enumerate() {
        if let (Some(xv), Some(o)) = (x.get_mut(v), old.get(k)) {
            *xv = *o;
        }
    }
}

/// Pushes the Jacobian rows of `eqs` at `x`, in order, into a row space; returns it and
/// which equations were dependent.
fn row_space(
    eqs: &[&Equation],
    n: usize,
    col: &[usize],
    x: &[f64],
) -> Result<(RowSpace, Vec<bool>), SolveError> {
    let (_, rows) = evaluate(eqs, col, x)?;
    let mut rs = RowSpace::new(n);
    let dependent = rows
        .iter()
        .map(|row| !rs.push(&dense_row(row, n)))
        .collect();
    Ok((rs, dependent))
}

fn owners(eqs: &[&Equation], pick: impl Fn(usize) -> bool) -> Vec<ConstraintId> {
    let set: BTreeSet<ConstraintId> = eqs
        .iter()
        .enumerate()
        .filter(|(k, _)| pick(*k))
        .filter_map(|(_, e)| e.owner)
        .collect();
    set.into_iter().collect()
}

/// How a failed group failed.
enum GroupFailure {
    Conflicting {
        conflicting: Vec<ConstraintId>,
        redundant: Vec<ConstraintId>,
    },
    NoSolution {
        unsatisfied: Vec<ConstraintId>,
        max_residual: f64,
    },
}

/// Solves one group in `x`. On failure, `x` is left with the group's unknowns as they were.
fn solve_group(
    sys: &System,
    group: &Group,
    x: &mut [f64],
) -> Result<Result<usize, GroupFailure>, SolveError> {
    let eqs = sys.equations(&group.eqs);
    if eqs.is_empty() {
        return Ok(Ok(0));
    }
    let start: Vec<f64> = x.to_vec();
    let out = levenberg_marquardt(&eqs, &group.vars, &sys.col, x)?;
    if out.converged {
        return Ok(Ok(out.iterations));
    }
    let failed_at = x.to_vec();
    x.copy_from_slice(&start);
    // Which rows depend on earlier ones, at the starting geometry?
    let (_, dependent) = row_space(&eqs, group.vars.len(), &sys.col, &start)?;
    if dependent.iter().any(|d| *d) {
        let independent: Vec<&Equation> = eqs
            .iter()
            .zip(&dependent)
            .filter(|(_, d)| !**d)
            .map(|(e, _)| *e)
            .collect();
        let mut trial = start.clone();
        let sub = levenberg_marquardt(&independent, &group.vars, &sys.col, &mut trial)?;
        if sub.converged {
            let res = residuals(&eqs, &trial);
            let bad = |k: usize| {
                dependent.get(k).copied().unwrap_or(false)
                    && res.get(k).is_none_or(|r| r.abs() > LINEAR_TOLERANCE)
            };
            let conflicting = owners(&eqs, bad);
            if conflicting.is_empty() && max_abs(&res) <= LINEAR_TOLERANCE {
                // Every dropped row holds: the reduced solve is a solution of the whole group.
                x.copy_from_slice(&trial);
                return Ok(Ok(sub.iterations));
            }
            if !conflicting.is_empty() {
                let mut redundant = owners(&eqs, |k| {
                    dependent.get(k).copied().unwrap_or(false) && !bad(k)
                });
                redundant.retain(|c| !conflicting.contains(c));
                return Ok(Err(GroupFailure::Conflicting {
                    conflicting,
                    redundant,
                }));
            }
        }
    }
    let res = residuals(&eqs, &failed_at);
    let unsatisfied = owners(&eqs, |k| {
        res.get(k).is_none_or(|r| r.abs() > LINEAR_TOLERANCE)
    });
    Ok(Err(GroupFailure::NoSolution {
        unsatisfied,
        max_residual: out.max_residual,
    }))
}

/// Degrees of freedom of the whole system at `x`.
fn analyse(sketch: &Sketch, sys: &System, x: &[f64]) -> Result<DofReport, SolveError> {
    let mut dof = 0;
    let mut redundant = BTreeSet::new();
    let mut determined = vec![false; x.len()];
    for group in &sys.groups {
        let eqs = sys.equations(&group.eqs);
        let n = group.vars.len();
        let (rs, dependent) = row_space(&eqs, n, &sys.col, x)?;
        dof += n - rs.rank();
        for (e, d) in eqs.iter().zip(&dependent) {
            if *d && let Some(o) = e.owner {
                redundant.insert(o);
            }
        }
        if rs.rank() > 0 {
            for (k, &v) in group.vars.iter().enumerate() {
                if rs.axis_freedom(k) <= DETERMINED_TOLERANCE
                    && let Some(slot) = determined.get_mut(v)
                {
                    *slot = true;
                }
            }
        }
    }
    let is = |i: usize| determined.get(i).copied().unwrap_or(false);
    let point_ok = |id: PointId| sys.layout.p(id).is_ok_and(|p| is(p) && is(p + 1));
    let determined_points: BTreeSet<PointId> = sys
        .layout
        .points()
        .filter(|(_, p)| is(*p) && is(p + 1))
        .map(|(id, _)| id)
        .collect();
    let radius_index: BTreeMap<CurveId, usize> = sys.layout.radii().collect();
    let determined_curves = sketch
        .curves()
        .filter(|(id, c)| {
            c.points().into_iter().all(point_ok) && radius_index.get(id).is_none_or(|r| is(*r))
        })
        .map(|(id, _)| id)
        .collect();
    Ok(DofReport {
        dof,
        status: if !redundant.is_empty() {
            ConstraintStatus::OverConstrained
        } else if dof == 0 {
            ConstraintStatus::FullyConstrained
        } else {
            ConstraintStatus::UnderConstrained
        },
        redundant: redundant.into_iter().collect(),
        determined_points,
        determined_curves,
    })
}

/// Writes the solved unknowns into a copy of the sketch and keeps it only if it is still
/// valid (finite, inside the working range, radii above the linear tolerance).
fn commit(sketch: &mut Sketch, layout: &Layout, x: &[f64]) -> Result<(), SolveError> {
    if x.iter().any(|v| !v.is_finite()) {
        return Err(SolveError::NonFinite);
    }
    let mut candidate = sketch.clone();
    write_back(&mut candidate, layout, x);
    candidate
        .validate()
        .map_err(|e| SolveError::Degenerate(e.to_string()))?;
    *sketch = candidate;
    Ok(())
}

fn write_back(sketch: &mut Sketch, layout: &Layout, x: &[f64]) {
    for (id, p) in layout.points() {
        if let (Some(px), Some(py)) = (x.get(p), x.get(p + 1)) {
            sketch.set_point_unchecked(id, Point2::new(*px, *py));
        }
    }
    for (id, r) in layout.radii() {
        if let Some(v) = x.get(r) {
            sketch.set_radius_unchecked(id, *v);
        }
    }
}

/// The unknowns of the groups containing any of `vars`, and those groups' equations.
fn merged_groups(sys: &System, vars: &[usize]) -> (Vec<usize>, Vec<usize>) {
    let mut gv = Vec::new();
    let mut ge = Vec::new();
    for g in &sys.groups {
        if vars.iter().any(|v| g.vars.binary_search(v).is_ok()) {
            gv.extend(&g.vars);
            ge.extend(&g.eqs);
        }
    }
    gv.sort_unstable();
    ge.sort_unstable();
    (gv, ge)
}

fn local_columns(len: usize, vars: &[usize]) -> Vec<usize> {
    let mut col = vec![usize::MAX; len];
    for (k, &v) in vars.iter().enumerate() {
        if let Some(slot) = col.get_mut(v) {
            *slot = k;
        }
    }
    col
}

/// Most halvings of a drag step before the drag stops where it is.
const MAX_HALVINGS: usize = 12;

fn distance_to(x: &[f64], px: usize, target: Point2) -> f64 {
    let p = Point2::new(
        x.get(px).copied().unwrap_or(f64::NAN),
        x.get(px + 1).copied().unwrap_or(f64::NAN),
    );
    p.distance(target)
}

/// The drag step at `x` (which satisfies `eqs`): the minimum-norm move that keeps `eqs` to
/// first order and brings unknowns `px`, `px + 1` as close to `target` as the null space of
/// the Jacobian allows.
fn drag_direction(
    eqs: &[&Equation],
    n: usize,
    col: &[usize],
    x: &[f64],
    px: usize,
    target: Point2,
) -> Result<Vec<f64>, SolveError> {
    let (Some(&cx), Some(&cy)) = (col.get(px), col.get(px + 1)) else {
        return Ok(vec![0.0; n]);
    };
    let (r, rows) = evaluate(eqs, col, x)?;
    let mut rs = RowSpace::new(n);
    let mut r_indep = Vec::new();
    for (row, ri) in rows.iter().zip(&r) {
        if rs.push(&dense_row(row, n)) {
            r_indep.push(-ri);
        }
    }
    let k = rs.rank();
    // With J_I = [Rᵀ 0] Qᵀ, write δ = Q w: the first k entries of w are fixed by
    // Rᵀ z = -r_I, the rest (the null space) are free.
    let z = rs.solve_rt(&r_indep).ok_or(SolveError::NonFinite)?;
    let axis = |c: usize| {
        let mut e = vec![0.0; n];
        if let Some(slot) = e.get_mut(c) {
            *slot = 1.0;
        }
        rs.apply_qt(&mut e);
        e
    };
    let (ax, ay) = (axis(cx), axis(cy));
    let head = |a: &[f64]| dot(a.get(..k).unwrap_or(&[]), &z);
    let want_x = target.x - x.get(px).copied().unwrap_or(0.0) - head(&ax);
    let want_y = target.y - x.get(px + 1).copied().unwrap_or(0.0) - head(&ay);
    let bx = ax.get(k..).unwrap_or(&[]);
    let by = ay.get(k..).unwrap_or(&[]);
    // Minimum-norm free part y with B y = want, B = [bxᵀ; byᵀ]: y = Bᵀ (B Bᵀ)⁺ want.
    let (m11, m12, m22) = (dot(bx, bx), dot(bx, by), dot(by, by));
    let tr = m11 + m22;
    let det = m11 * m22 - m12 * m12;
    let (s0, s1) = if tr <= 1e-12 {
        (0.0, 0.0)
    } else if det > 1e-12 * tr * tr {
        (
            (m22 * want_x - m12 * want_y) / det,
            (m11 * want_y - m12 * want_x) / det,
        )
    } else {
        // Rank one: M⁺ = M / tr².
        let t2 = tr * tr;
        (
            (m11 * want_x + m12 * want_y) / t2,
            (m12 * want_x + m22 * want_y) / t2,
        )
    };
    let mut w = z;
    w.extend(bx.iter().zip(by).map(|(u, v)| u * s0 + v * s1));
    rs.apply_q(&mut w);
    if w.iter().all(|v| v.is_finite()) {
        Ok(w)
    } else {
        Err(SolveError::NonFinite)
    }
}

/// Moves unknowns `px`, `px + 1` toward `target` while keeping `eqs`: each step follows
/// [`drag_direction`], is projected back onto the constraints by Levenberg-Marquardt, and
/// is accepted only if the point got closer to the target (otherwise it is halved). `x`
/// must satisfy `eqs` on entry and does on exit. Returns the steps taken.
fn drag_steps(
    eqs: &[&Equation],
    vars: &[usize],
    col: &[usize],
    x: &mut [f64],
    px: usize,
    target: Point2,
    scale: f64,
) -> Result<usize, SolveError> {
    let n = vars.len();
    let mut steps = 0;
    let mut trial = x.to_vec();
    let mut w = drag_direction(eqs, n, col, x, px, target)?;
    while steps < MAX_DRAG_STEPS {
        steps += 1;
        let len = norm(&w);
        if len <= SOLVE_TARGET {
            break;
        }
        let here = distance_to(x, px, target);
        let mut alpha = if len > scale { scale / len } else { 1.0 };
        let mut accepted = None;
        for _ in 0..MAX_HALVINGS {
            trial.copy_from_slice(x);
            for (k, &v) in vars.iter().enumerate() {
                if let (Some(t), Some(d)) = (trial.get_mut(v), w.get(k)) {
                    *t += alpha * d;
                }
            }
            let projected = levenberg_marquardt(eqs, vars, col, &mut trial)?;
            let flipped = eqs.iter().any(|e| e.residual.flips(x, &trial));
            if projected.converged && !flipped {
                // Progress is either a shorter distance to the target, or a smaller movable
                // part of the drag direction (it vanishes at the closest reachable place, and
                // unlike the distance it changes to first order there).
                let w_trial = drag_direction(eqs, n, col, &trial, px, target)?;
                if distance_to(&trial, px, target) < here || norm(&w_trial) < len {
                    accepted = Some(w_trial);
                    break;
                }
            }
            alpha *= 0.5;
        }
        let Some(w_next) = accepted else {
            break;
        };
        let moved = Point2::new(
            trial.get(px).copied().unwrap_or(0.0),
            trial.get(px + 1).copied().unwrap_or(0.0),
        )
        .distance(Point2::new(
            x.get(px).copied().unwrap_or(0.0),
            x.get(px + 1).copied().unwrap_or(0.0),
        ));
        x.copy_from_slice(&trial);
        w = w_next;
        if moved <= SOLVE_TARGET {
            break;
        }
    }
    Ok(steps)
}

impl Sketch {
    /// Solves every constraint, starting from the current geometry. On success the new
    /// geometry is written back and the degrees of freedom are reported; on failure the
    /// sketch is unchanged.
    pub fn solve(&mut self) -> Result<SolveReport, SolveError> {
        let sys = System::build(self)?;
        let mut x = sys.layout.x.clone();
        let mut iterations = 0;
        let mut conflicting = BTreeSet::new();
        let mut redundant = BTreeSet::new();
        let mut unsatisfied = BTreeSet::new();
        let mut worst: f64 = 0.0;
        for group in &sys.groups {
            match solve_group(&sys, group, &mut x)? {
                Ok(it) => iterations = iterations.max(it),
                Err(GroupFailure::Conflicting {
                    conflicting: c,
                    redundant: r,
                }) => {
                    conflicting.extend(c);
                    redundant.extend(r);
                }
                Err(GroupFailure::NoSolution {
                    unsatisfied: u,
                    max_residual,
                }) => {
                    unsatisfied.extend(u);
                    worst = worst.max(max_residual);
                }
            }
        }
        if !conflicting.is_empty() {
            return Err(SolveError::Conflicting {
                conflicting: conflicting.into_iter().collect(),
                redundant: redundant.into_iter().collect(),
            });
        }
        if !unsatisfied.is_empty() || worst > LINEAR_TOLERANCE {
            return Err(SolveError::NoSolution {
                unsatisfied: unsatisfied.into_iter().collect(),
                max_residual: worst,
            });
        }
        let all: Vec<&Equation> = sys.eqs.iter().collect();
        let max_residual = max_abs(&residuals(&all, &x));
        let dof = analyse(self, &sys, &x)?;
        commit(self, &sys.layout, &x)?;
        Ok(SolveReport {
            iterations,
            max_residual,
            dof,
        })
    }

    /// Degrees of freedom at the current geometry, without solving.
    pub fn analyze(&self) -> Result<DofReport, SolveError> {
        let sys = System::build(self)?;
        analyse(self, &sys, &sys.layout.x)
    }

    /// Adds a constraint and solves. If the solve fails the constraint is not added and the
    /// sketch is unchanged.
    pub fn try_add_constraint(
        &mut self,
        c: crate::Constraint,
    ) -> Result<(ConstraintId, SolveReport), SolveError> {
        let backup = self.clone();
        let id = self.add_constraint(c)?;
        match self.solve() {
            Ok(report) => Ok((id, report)),
            Err(e) => {
                // Restore everything, including the id counter.
                *self = backup;
                Err(e)
            }
        }
    }

    /// Changes a dimension and solves. If the solve fails the old value is kept and the
    /// sketch is unchanged.
    pub fn try_set_dimension(
        &mut self,
        id: ConstraintId,
        value: f64,
    ) -> Result<SolveReport, SolveError> {
        let backup = self.clone();
        self.set_dimension(id, value)?;
        match self.solve() {
            Ok(report) => Ok(report),
            Err(e) => {
                *self = backup;
                Err(e)
            }
        }
    }

    /// Moves `point` toward `target` while keeping every constraint. Only the geometry
    /// coupled to the point moves. A point that cannot reach the target stops at the nearest
    /// place it can reach. On failure the sketch is unchanged.
    pub fn drag(&mut self, point: PointId, target: Point2) -> Result<DragReport, SolveError> {
        if !(is_valid_coordinate(target.x) && is_valid_coordinate(target.y)) {
            return Err(SketchError::BadCoordinate {
                what: "target",
                value: if target.x.is_finite() {
                    target.y
                } else {
                    target.x
                },
            }
            .into());
        }
        let sys = System::build(self)?;
        let px = sys.layout.p(point)?;
        let (vars, eq_ids) = merged_groups(&sys, &[px, px + 1]);
        let eqs = sys.equations(&eq_ids);
        let col = local_columns(sys.layout.x.len(), &vars);
        let mut x = sys.layout.x.clone();
        let mut steps = 0;
        let mut ok = false;
        if eqs.is_empty() {
            set_xy(&mut x, px, target);
            ok = true;
        } else {
            let scale = sys.layout.scale;
            // Start from geometry that satisfies the constraints.
            let start = levenberg_marquardt(&eqs, &vars, &col, &mut x)?;
            steps += start.iterations;
            if start.converged {
                steps += drag_steps(&eqs, &vars, &col, &mut x, px, target, scale)?;
                ok = true;
            }
        }
        if !ok {
            let res = residuals(&eqs, &x);
            return Err(SolveError::NoSolution {
                unsatisfied: owners(&eqs, |k| {
                    res.get(k).is_none_or(|r| r.abs() > LINEAR_TOLERANCE)
                }),
                max_residual: max_abs(&res),
            });
        }
        commit(self, &sys.layout, &x)?;
        let reached = Point2::new(
            x.get(px).copied().unwrap_or(f64::NAN),
            x.get(px + 1).copied().unwrap_or(f64::NAN),
        );
        Ok(DragReport {
            steps,
            reached,
            distance_to_target: reached.distance(target),
        })
    }
}

fn set_xy(x: &mut [f64], px: usize, p: Point2) {
    if let Some(v) = x.get_mut(px) {
        *v = p.x;
    }
    if let Some(v) = x.get_mut(px + 1) {
        *v = p.y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Constraint;

    #[test]
    fn unknowns_split_into_independent_groups() {
        // A horizontal line: its two y coordinates are coupled, each x is alone.
        let mut s = Sketch::new();
        let a = s.add_point(Point2::new(0.0, 0.0)).unwrap();
        let b = s.add_point(Point2::new(5.0, 1.0)).unwrap();
        let l = s.add_line(a, b).unwrap();
        s.add_constraint(Constraint::Horizontal { line: l })
            .unwrap();
        let sys = System::build(&s).unwrap();
        let mut sizes: Vec<usize> = sys.groups.iter().map(|g| g.vars.len()).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, vec![1, 1, 2]);
    }

    #[test]
    fn one_equation_over_many_unknowns_makes_one_group() {
        // A point on a circle couples the point, the centre and the radius: 5 unknowns.
        // (Regression: merging must use fresh roots, or earlier links are overwritten.)
        let mut s = Sketch::new();
        let c = s.add_point(Point2::new(0.0, 0.0)).unwrap();
        let k = s.add_circle(c, 10.0).unwrap();
        let p = s.add_point(Point2::new(10.0, 0.0)).unwrap();
        s.add_constraint(Constraint::PointOnCircle { point: p, curve: k })
            .unwrap();
        s.fix_point(c).unwrap();
        let sys = System::build(&s).unwrap();
        assert_eq!(sys.groups.len(), 1);
        assert_eq!(sys.groups[0].vars.len(), 5);
        assert_eq!(sys.groups[0].eqs.len(), 3);
    }

    #[test]
    fn levenberg_marquardt_finds_the_nearest_root() {
        // |p - q| = 5 with q fixed at the origin and p starting at (3, 1): the nearest
        // solution is along the same direction, 5 (3, 1) / √10.
        let mut s = Sketch::new();
        let q = s.add_point(Point2::new(0.0, 0.0)).unwrap();
        let p = s.add_point(Point2::new(3.0, 1.0)).unwrap();
        s.fix_point(q).unwrap();
        s.add_constraint(Constraint::Distance {
            a: p,
            b: q,
            value: 5.0,
        })
        .unwrap();
        s.solve().unwrap();
        let got = s.point(p).unwrap();
        let k = 5.0 / 10f64.sqrt();
        assert!((got.x - 3.0 * k).abs() < 1e-9 && (got.y - k).abs() < 1e-9);
    }

    #[test]
    fn a_failed_solve_reports_and_keeps_the_start() {
        // Two fixes of the same point at different places conflict.
        let mut s = Sketch::new();
        let p = s.add_point(Point2::new(1.0, 1.0)).unwrap();
        s.fix_point(p).unwrap();
        let second = s
            .add_constraint(Constraint::Fix {
                point: p,
                at: Point2::new(2.0, 1.0),
            })
            .unwrap();
        let before = s.clone();
        match s.solve() {
            Err(SolveError::Conflicting { conflicting, .. }) => {
                assert_eq!(conflicting, vec![second]);
            }
            other => panic!("expected a conflict, got {other:?}"),
        }
        assert_eq!(s, before);
    }
}
