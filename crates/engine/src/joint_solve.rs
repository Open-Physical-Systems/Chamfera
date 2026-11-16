//! Placing components from their joints: the tree walk, as-built joints, and loops.
//!
//! The joints (and rigid group links) are walked as a tree from the grounded components
//! (`chamfera_doc::joint::topology`); each tree joint places its far component exactly
//! from the near one. A joint the tree does not use closes a loop: for each loop group the
//! values of its joints that are not `driven` are solved ([`solve_loop`]) with
//! Levenberg-Marquardt (`crate::loop_solver`) so every closing joint fits, then the tree
//! places everything at those values.

use crate::assembly::{JointValues, LoopReport, Placements, joint_relative};
use chamfera_doc::component::ROOT;
use chamfera_doc::{Assembly, Joint, JointKind, JointOrigin};
use chamfera_geom::{Frame, Point3, Transform};
use std::collections::BTreeMap;

/// Each joint's two origin frames (on A and on B, each in its component's coordinates).
type Frames = BTreeMap<u32, (Frame, Frame)>;

/// Most unknowns one loop group may have.
pub const MAX_LOOP_UNKNOWNS: usize = 120;

/// True when two transforms differ by no more than rounding: an as-built joint at its
/// as-built value then keeps its component's own placement bit for bit, instead of the
/// same place recomposed through the joint's frames.
fn same_place(a: &Transform, b: &Transform) -> bool {
    let (ma, mb) = (a.matrix(), b.matrix());
    let turn = ma
        .iter()
        .flatten()
        .zip(mb.iter().flatten())
        .all(|(x, y)| (x - y).abs() <= 1e-12);
    let (ta, tb) = (a.translation_part(), b.translation_part());
    let scale = 1.0 + ta.length().max(tb.length());
    turn && (ta - tb).length() <= 1e-12 * scale
}

/// Places every component from the joints (at their current values) along the tree
/// `drivers`, and the rest from their placements. A joint without frames leaves its
/// component at its own placement. `snap` lets an as-built joint keep its component's
/// own placement when the joint puts it there.
fn place(
    asm: &Assembly,
    joints: &[Joint],
    drivers: &BTreeMap<u32, chamfera_doc::joint::Driver>,
    frames: &Frames,
    snap: bool,
    errors: &mut BTreeMap<u32, String>,
) -> BTreeMap<u32, Transform> {
    let mut world: BTreeMap<u32, Transform> = BTreeMap::new();
    world.insert(ROOT, Transform::IDENTITY);
    // What each component's position depends on: a joint and the component it is reached
    // from (and whether it may snap to its own placement), or its own placement inside
    // its parent.
    enum Rule {
        Joint(Transform, u32, bool),
        Parent(Transform, u32),
    }
    let mut rules: Vec<(u32, Rule)> = Vec::new();
    for c in &asm.components {
        let own = Rule::Parent(c.placement, c.parent);
        let rule = match drivers.get(&c.id) {
            None => own,
            Some(d) => {
                let Some((fa, fb)) = frames.get(&d.joint) else {
                    rules.push((c.id, own));
                    continue;
                };
                let made = joints.iter().find(|j| j.id == d.joint).and_then(|j| {
                    let rel = joint_relative(j, fa, fb)?;
                    let t = if d.forward {
                        rel
                    } else {
                        rel.inverse_rigid()?
                    };
                    Some((t, j.as_built))
                });
                match made {
                    Some((t, as_built)) => Rule::Joint(t, d.from, as_built && snap),
                    None => {
                        errors
                            .entry(d.joint)
                            .or_insert_with(|| "its origins do not make a proper move".into());
                        own
                    }
                }
            }
        };
        rules.push((c.id, rule));
    }
    // Resolve in passes: each pass places every component whose reference is placed. A
    // tree needs at most one pass per level; anything left after that depends on itself
    // (a joint against the component's own parent chain) and keeps its plain placement.
    // An as-built joint waits for its component's parent too, to compare with it; when
    // the parent never comes, a relaxed pass places it by the joint alone.
    let mut relaxed = false;
    for _ in 0..=(2 * rules.len() + 1) {
        let mut progress = false;
        for (id, rule) in &rules {
            if world.contains_key(id) {
                continue;
            }
            let placed = match rule {
                Rule::Parent(t, from) => world.get(from).map(|base| t.then(base)),
                Rule::Joint(t, from, as_built) => match world.get(from).map(|b| t.then(b)) {
                    Some(by_joint) if *as_built => {
                        let own = asm
                            .component(*id)
                            .ok()
                            .and_then(|c| world.get(&c.parent).map(|p| c.placement.then(p)));
                        match own {
                            Some(own) if same_place(&own, &by_joint) => Some(own),
                            Some(_) => Some(by_joint),
                            None if relaxed => Some(by_joint),
                            None => None,
                        }
                    }
                    other => other,
                },
            };
            if let Some(t) = placed {
                world.insert(*id, t);
                progress = true;
            }
        }
        if !progress {
            if relaxed {
                break;
            }
            relaxed = true;
        }
    }
    for (id, rule) in &rules {
        if !world.contains_key(id) {
            if let (Rule::Joint(..), Some(d)) = (rule, drivers.get(id)) {
                errors.insert(
                    d.joint,
                    "it positions a component against that component's own parent; \
                     joint it to another component"
                        .into(),
                );
            }
            let own = asm
                .component(*id)
                .map_or(Transform::IDENTITY, |c| c.placement);
            world.insert(*id, own);
        }
    }
    world
}

/// Solves every component's world transform. `frame` finds a joint origin's frame in its
/// component's own coordinates.
pub fn solve(asm: &Assembly, frame: &dyn Fn(&JointOrigin) -> Result<Frame, String>) -> Placements {
    let mut out = Placements::default();
    let mut joints = asm.solving_joints();
    let topo = chamfera_doc::joint::topology(asm, &joints);
    let mut frames: Frames = BTreeMap::new();
    for j in &joints {
        let fa = frame(&j.a).map_err(|e| format!("its origin on A: {e}"));
        let fb = frame(&j.b).map_err(|e| format!("its origin on B: {e}"));
        match (fa, fb) {
            (Ok(fa), Ok(fb)) => {
                frames.insert(j.id, (fa, fb));
            }
            (Err(e), _) | (_, Err(e)) => {
                out.errors.insert(j.id, e);
            }
        }
    }
    for group in &topo.loops {
        let report = solve_loop(asm, &topo, group, &mut joints, &frames);
        if let Some(e) = &report.error {
            for id in group {
                out.errors.entry(*id).or_insert_with(|| e.clone());
            }
        } else {
            for j in joints.iter().filter(|j| group.contains(&j.id) && !j.driven) {
                if j.id <= asm.last_id {
                    out.solved.insert(
                        j.id,
                        JointValues {
                            value: j.value,
                            distance: j.distance,
                            offset: j.offset,
                        },
                    );
                }
            }
        }
        out.loops.push(report);
    }
    out.world = place(asm, &joints, &topo.drivers, &frames, true, &mut out.errors);
    // Rigid group links report under their group.
    let links: Vec<u32> = out
        .errors
        .keys()
        .copied()
        .filter(|id| *id > asm.last_id)
        .collect();
    for id in links {
        if let Some(e) = out.errors.remove(&id)
            && let Some(g) = asm.link_group(id)
        {
            out.group_errors.entry(g).or_insert(e);
        }
    }
    out
}

/// One unknown of a loop: a joint (by index into the solving joints) and which value.
#[derive(Debug, Clone, Copy)]
enum Dof {
    Value(usize),
    Distance(usize),
    OffsetX(usize),
    OffsetY(usize),
}

impl Dof {
    fn get(self, joints: &[Joint]) -> f64 {
        match self {
            Dof::Value(i) => joints.get(i).map_or(0.0, |j| j.value),
            Dof::Distance(i) => joints.get(i).map_or(0.0, |j| j.distance),
            Dof::OffsetX(i) => joints.get(i).map_or(0.0, |j| j.offset[0]),
            Dof::OffsetY(i) => joints.get(i).map_or(0.0, |j| j.offset[1]),
        }
    }

    fn set(self, joints: &mut [Joint], v: f64) {
        let (i, slot): (usize, fn(&mut Joint) -> &mut f64) = match self {
            Dof::Value(i) => (i, |j| &mut j.value),
            Dof::Distance(i) => (i, |j| &mut j.distance),
            Dof::OffsetX(i) => (i, |j| &mut j.offset[0]),
            Dof::OffsetY(i) => (i, |j| &mut j.offset[1]),
        };
        if let Some(j) = joints.get_mut(i) {
            *slot(j) = v;
        }
    }

    fn is_angle(self, joints: &[Joint]) -> bool {
        matches!(self, Dof::Value(i) if joints.get(i).is_some_and(|j| {
            j.kind.value_unit() == Some(chamfera_doc::Unit::Angle)
        }))
    }
}

/// The values a joint of kind `kind` (at index `i`) moves by.
fn dofs_of(i: usize, kind: JointKind) -> Vec<Dof> {
    match kind {
        JointKind::Rigid => Vec::new(),
        JointKind::Revolute | JointKind::Slider => vec![Dof::Value(i)],
        JointKind::Cylindrical => vec![Dof::Value(i), Dof::Distance(i)],
        JointKind::Planar => vec![Dof::Value(i), Dof::OffsetX(i), Dof::OffsetY(i)],
    }
}

/// Residuals per closing joint: where the joint puts B's origin minus where the tree put
/// it (3 values, mm), the turn between the two as a rotation vector, and the shortfall of
/// the turn's trace (so a half turn, which has no rotation vector part, is not taken as
/// closed); turns are times `scale` so they weigh as mm at the loop's size.
fn closure(
    asm: &Assembly,
    joints: &[Joint],
    topo: &chamfera_doc::Topology,
    closing: &[u32],
    frames: &Frames,
    scale: f64,
) -> Option<Vec<f64>> {
    let mut scratch = BTreeMap::new();
    let world = place(asm, joints, &topo.drivers, frames, false, &mut scratch);
    let mut r = Vec::with_capacity(7 * closing.len());
    for id in closing {
        let j = joints.iter().find(|j| j.id == *id)?;
        let (fa, fb) = frames.get(id)?;
        let rel = joint_relative(j, fa, fb)?;
        let by_joint = rel.then(world.get(&j.a.component)?);
        let tree = world.get(&j.b.component)?;
        let d = by_joint.translation_part() - tree.translation_part();
        // E = P Qᵀ (the turn from the tree's place to the joint's); its skew part is
        // sin(angle) times the axis, its trace 1 + 2 cos(angle).
        let (p, q) = (by_joint.matrix(), tree.matrix());
        let e = |i: usize, k: usize| -> f64 { (0..3).map(|c| p[i][c] * q[k][c]).sum() };
        let trace = e(0, 0) + e(1, 1) + e(2, 2);
        r.extend([d.x, d.y, d.z]);
        r.extend([
            0.5 * (e(2, 1) - e(1, 2)) * scale,
            0.5 * (e(0, 2) - e(2, 0)) * scale,
            0.5 * (e(1, 0) - e(0, 1)) * scale,
        ]);
        r.push(0.5 * (3.0 - trace).max(0.0) * scale);
    }
    r.iter().all(|v| v.is_finite()).then_some(r)
}

/// Joint names for messages: up to six, then how many more.
fn names(asm: &Assembly, joints: &[Joint], ids: &[u32]) -> String {
    let mut shown: Vec<String> = ids
        .iter()
        .take(6)
        .map(|id| match asm.link_group(*id) {
            Some(g) => format!(
                "rigid group {:?}",
                asm.group(g)
                    .map_or_else(|_| g.to_string(), |g| g.name.clone())
            ),
            None => joints
                .iter()
                .find(|j| j.id == *id)
                .map_or_else(|| id.to_string(), |j| format!("{:?}", j.name)),
        })
        .collect();
    shown.dedup();
    if ids.len() > 6 {
        shown.push(format!("{} more", ids.len() - 6));
    }
    shown.join(", ")
}

/// Solves one loop group: the values of its joints that are not driven, so that its
/// closing joints fit. On success the joints hold the solved values; on failure they
/// keep the values they had and the report says why, naming the joints.
pub fn solve_loop(
    asm: &Assembly,
    topo: &chamfera_doc::Topology,
    group: &[u32],
    joints: &mut [Joint],
    frames: &Frames,
) -> LoopReport {
    let closing: Vec<u32> = topo
        .closing
        .iter()
        .copied()
        .filter(|c| group.contains(c))
        .collect();
    let mut report = LoopReport {
        joints: group.to_vec(),
        unknowns: 0,
        equations: 7 * closing.len(),
        freedom: 0,
        residual: 0.0,
        iterations: 0,
        error: None,
    };
    let listed = names(asm, joints, group);
    if group.iter().any(|id| !frames.contains_key(id)) {
        report.error = Some(format!(
            "the loop through {listed} cannot be solved: one of its joint origins is missing"
        ));
        return report;
    }
    let mut free: Vec<Dof> = Vec::new();
    let mut all: Vec<Dof> = Vec::new();
    for (i, j) in joints.iter().enumerate() {
        if group.contains(&j.id) {
            let d = dofs_of(i, j.kind);
            all.extend(d.iter().copied());
            if !j.driven {
                free.extend(d);
            }
        }
    }
    report.unknowns = free.len();
    if all.len() > MAX_LOOP_UNKNOWNS {
        report.error = Some(format!(
            "the loop through {listed} has {} joint values; at most {MAX_LOOP_UNKNOWNS} \
             are solved together",
            all.len()
        ));
        return report;
    }
    // Turns count as mm at the size of the loop, so both kinds of error weigh alike.
    let scale = 1.0
        + group
            .iter()
            .filter_map(|id| frames.get(id))
            .flat_map(|(a, b)| [a.origin(), b.origin()])
            .map(|p| (p - Point3::ORIGIN).length())
            .fold(0.0_f64, f64::max)
            .min(chamfera_geom::MAX_COORDINATE);
    let m = report.equations;
    let base: Vec<Joint> = joints.to_vec();
    let residuals = |dofs: &[Dof], x: &[f64]| -> Option<Vec<f64>> {
        let mut trial = base.clone();
        for (d, v) in dofs.iter().zip(x) {
            d.set(&mut trial, *v);
        }
        closure(asm, &trial, topo, &closing, frames, scale)
    };
    let start: Vec<f64> = free.iter().map(|d| d.get(joints)).collect();
    let f = |x: &[f64]| residuals(&free, x);
    let mut x = start.clone();
    let outcome = crate::loop_solver::levenberg_marquardt(&f, &mut x, m);
    // The freedom left: unknowns the closing equations do not fix.
    report.freedom = crate::loop_solver::jacobian(&f, &x, m).map_or(0, |jac| {
        free.len() - crate::loop_solver::rank(&jac, m, free.len())
    });
    report.iterations = outcome.iterations;
    report.residual = outcome.max_residual;
    if outcome.max_residual <= chamfera_geom::LINEAR_TOLERANCE {
        for ((d, v), s) in free.iter().zip(&x).zip(&start) {
            // An angle keeps the turn nearest to where it was, so values stay continuous.
            let v = if d.is_angle(joints) {
                let pi = std::f64::consts::PI;
                s + (v - s + pi).rem_euclid(std::f64::consts::TAU) - pi
            } else {
                *v
            };
            d.set(joints, v);
        }
        let outside = joints
            .iter()
            .find(|j| group.contains(&j.id) && !j.driven && j.check_value(j.value).is_err());
        if let Some(j) = outside {
            report.error = Some(format!(
                "the loop through {listed} needs {:?} at {}, outside its limits",
                j.name,
                chamfera_doc::joint::show_value(j.kind, j.value)
            ));
            for (d, s) in free.iter().zip(&start) {
                d.set(joints, *s);
            }
        }
        return report;
    }
    // It does not close. Over-constrained when more values are driven than the loop can
    // move, or when it cannot move at all; otherwise the driven values are out of reach.
    let driven: Vec<u32> = joints
        .iter()
        .filter(|j| group.contains(&j.id) && j.driven && j.kind != JointKind::Rigid)
        .map(|j| j.id)
        .collect();
    let driven_count = all.len() - free.len();
    let at_all: Vec<f64> = all.iter().map(|d| d.get(joints)).collect();
    let mobility = crate::loop_solver::jacobian(&|x: &[f64]| residuals(&all, x), &at_all, m)
        .map_or(0, |jac| {
            all.len() - crate::loop_solver::rank(&jac, m, all.len())
        });
    let off = format!("{:.4} mm", outcome.max_residual);
    report.error = Some(if driven_count > 0 && driven_count > mobility {
        format!(
            "the loop through {listed} is over-constrained: it can move {mobility} way(s) \
             but {driven_count} value(s) are driven ({}), and they disagree by {off}. Let \
             one be solved (Edit Joint, or joint.edit with driven: false)",
            names(asm, joints, &driven)
        )
    } else if mobility == 0 {
        format!(
            "the loop through {listed} is over-constrained: its joints leave it no motion \
             and do not fit together (off by {off}). Delete a joint, or make one as-built \
             where the parts sit"
        )
    } else {
        format!(
            "the loop through {listed} cannot close at these values (off by {off}): the \
             parts do not reach. Drive {} to another value",
            if driven.is_empty() {
                "a joint".to_owned()
            } else {
                names(asm, joints, &driven)
            }
        )
    });
    report
}
