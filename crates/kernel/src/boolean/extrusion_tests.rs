//! Booleans on solids with spline sides (surfaces of linear extrusion, rung K10 on K7).
//! Expected values are closed forms derived in the comments.

use crate::{
    Body, KernelError, Profile, ProfileEdge, TessOptions, extrude, intersect, subtract, tessellate,
    union,
};
use chamfera_geom::{BSplineCurve2, Frame, Point2, Point3, Transform, Vec3};
use std::collections::HashMap;
use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, PI};

fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1.0)
}

/// The ellipse x²/a² + y²/b² = 1 about `c`, turned by `angle`, as two rational quadratic
/// halves: the nine-point rational circle (The NURBS Book, §7.3) with its control points
/// scaled by (a, b) and turned (an affine image of a NURBS curve is the NURBS curve of the
/// mapped control points).
fn ellipse(c: Point2, a: f64, b: f64, angle: f64) -> Profile {
    let w = FRAC_1_SQRT_2;
    let corners = [
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (-1.0, 1.0),
        (-1.0, 0.0),
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (1.0, 0.0),
    ];
    let (s, co) = angle.sin_cos();
    let curve = BSplineCurve2::new(
        2,
        vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ],
        corners
            .iter()
            .map(|(x, y)| {
                let (x, y) = (a * x, b * y);
                pt(c.x + co * x - s * y, c.y + s * x + co * y)
            })
            .collect(),
        Some(vec![1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0]),
    )
    .unwrap();
    let (h0, h1) = curve.split_at(0.5).unwrap();
    Profile::new(
        vec![ProfileEdge::Spline(h0), ProfileEdge::Spline(h1)],
        vec![],
    )
}

/// The arch of the splines work: the fit spline through (0, 0), (10, 10), (20, 0), the
/// parabola y = x (20 - x) / 10, closed by a line. Its area is
/// ∫₀²⁰ x (20 - x) / 10 dx = (20 · 20²/2 - 20³/3) / 10 = 400/3.
fn arch() -> Profile {
    let s = BSplineCurve2::interpolate(&[pt(0.0, 0.0), pt(10.0, 10.0), pt(20.0, 0.0)], None, None)
        .unwrap();
    Profile::new(
        vec![
            ProfileEdge::Spline(s),
            ProfileEdge::line(pt(20.0, 0.0), pt(0.0, 0.0)),
        ],
        vec![],
    )
}

/// A box from `lo` to `hi`.
fn block(lo: Point3, hi: Point3) -> Body {
    let f = Frame::XY.with_origin(Point3::new(0.0, 0.0, lo.z)).unwrap();
    let p = Profile::polygon(&[
        pt(lo.x, lo.y),
        pt(hi.x, lo.y),
        pt(hi.x, hi.y),
        pt(lo.x, hi.y),
    ]);
    extrude(&f, &p, hi.z - lo.z).unwrap()
}

/// The ellipse plate: a = 10, b = 5 extruded h = 10, volume π a b h = 500π.
fn plate() -> Body {
    extrude(&Frame::XY, &ellipse(Point2::ORIGIN, 10.0, 5.0, 0.0), 10.0).unwrap()
}

/// Validity, a closed consistently oriented mesh whose volume is within the chord
/// tolerance of `volume`, and the exact volume to the quadrature tolerance.
fn check(body: &Body, volume: f64) {
    body.validate().unwrap();
    let v = body.volume().unwrap();
    assert!(close(v, volume, 1e-9), "volume {v} vs {volume}");
    let opts = TessOptions::default();
    let m = tessellate(body, &opts).unwrap();
    let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
    for t in &m.mesh.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *directed.entry((a, b)).or_default() += 1;
        }
    }
    for (&(a, b), &n) in &directed {
        assert_eq!(n, 1, "edge {a}->{b} used {n} times");
        assert_eq!(directed.get(&(b, a)), Some(&1), "edge {a}->{b} unpaired");
    }
    let area = body.area().unwrap();
    let mv = m.mesh.signed_volume();
    assert!(
        (mv - volume).abs() <= area * opts.chord_tolerance,
        "mesh volume {mv} vs {volume}"
    );
}

#[test]
fn half_an_ellipse_plate_is_cut_off_by_a_box() {
    // The box x >= 0 covers half the plate (the ellipse is symmetric in x): 250π. The box
    // runs past the plate in z, so only the plane x = 0 cuts the elliptic sides, along two
    // rulings at (0, ±5).
    let p = plate();
    let b = block(Point3::new(0.0, -20.0, -1.0), Point3::new(30.0, 20.0, 11.0));
    let left = subtract(&p, &b).unwrap();
    check(&left, 250.0 * PI);
    let right = intersect(&p, &b).unwrap();
    check(&right, 250.0 * PI);
    // Flush with both caps: the box's top and bottom coincide with the plate's caps and
    // imprint on them.
    let flush = block(Point3::new(0.0, -20.0, 0.0), Point3::new(30.0, 20.0, 10.0));
    check(&subtract(&p, &flush).unwrap(), 250.0 * PI);
    check(&intersect(&p, &flush).unwrap(), 250.0 * PI);
    // The union with the flush box is the box plus the left half: 30·40·10 + 250π.
    check(&union(&p, &flush).unwrap(), 12_000.0 + 250.0 * PI);
}

#[test]
fn an_ellipse_plate_with_a_centred_hole() {
    // A cylinder r = 2 through the plate's centre: 500π - π 2² 10 = 460π.
    let hole = extrude(
        &Frame::XY.with_origin(Point3::new(0.0, 0.0, -1.0)).unwrap(),
        &Profile::circle(Point2::ORIGIN, 2.0),
        12.0,
    )
    .unwrap();
    let p = subtract(&plate(), &hole).unwrap();
    check(&p, 460.0 * PI);
    // A blind hole 4 deep from the top: 500π - 16π.
    let blind = extrude(
        &Frame::XY.with_origin(Point3::new(0.0, 0.0, 6.0)).unwrap(),
        &Profile::circle(Point2::ORIGIN, 2.0),
        5.0,
    )
    .unwrap();
    check(&subtract(&plate(), &blind).unwrap(), 484.0 * PI);
}

#[test]
fn two_crossed_ellipses_share_four_a_b_atan_b_over_a() {
    // E1: x²/a² + y²/b² <= 1 and E2 the same turned 90°: x²/b² + y²/a² <= 1, a > b. In
    // polar coordinates E1 reaches r1(θ)² = 1 / (cos²θ/a² + sin²θ/b²) and E2
    // r2(θ)² = 1 / (cos²θ/b² + sin²θ/a²); for 0 <= θ <= π/4, r2 <= r1. By the eight-fold
    // symmetry the common area is
    //   A = 8 ∫₀^{π/4} ½ r2² dθ = 4 ∫₀^{π/4} dθ / (cos²θ/b² + sin²θ/a²),
    // and with t = tan θ (dθ = dt / (1 + t²), cos²θ = 1 / (1 + t²)):
    //   A = 4 ∫₀¹ dt / (1/b² + t²/a²) = 4 a² ∫₀¹ dt / (a²/b² + t²) = 4 a b atan(b / a).
    // With a = 10, b = 5 and h = 10: V = 10 · 200 atan(1/2) = 2000 atan(1/2).
    let e1 = plate();
    let e2 = extrude(
        &Frame::XY,
        &ellipse(Point2::ORIGIN, 10.0, 5.0, FRAC_PI_2),
        10.0,
    )
    .unwrap();
    let common = 2000.0 * 0.5f64.atan();
    check(&intersect(&e1, &e2).unwrap(), common);
    // Union: V1 + V2 - V(common); difference: V1 - V(common).
    check(&union(&e1, &e2).unwrap(), 1000.0 * PI - common);
    check(&subtract(&e1, &e2).unwrap(), 500.0 * PI - common);
}

#[test]
fn a_spline_plate_cut_at_half_height_keeps_half() {
    // The arch (area 400/3) extruded 10, minus everything above z = 5: (400/3) · 5.
    let body = extrude(&Frame::XY, &arch(), 10.0).unwrap();
    let top = block(Point3::new(-5.0, -5.0, 5.0), Point3::new(25.0, 15.0, 20.0));
    let half = 2000.0 / 3.0;
    check(&subtract(&body, &top).unwrap(), half);
    check(&intersect(&body, &top).unwrap(), half);
}

#[test]
fn an_oblique_plane_cuts_an_ellipse_plate_by_cavalieri() {
    // The half-space z <= 5 + x / 4 (a plane tilted along x) meets the plate's sides in an
    // oblique section, an exact B-spline. Every vertical line through the ellipse keeps
    // height 5 + x / 4 (between 2.5 and 7.5, inside the plate), so the volume is
    // ∬ (5 + x/4) dA = 5 · 50π + 0 (the ellipse is symmetric in x) = 250π.
    let p = plate();
    let tool = block(
        Point3::new(-30.0, -30.0, -10.0),
        Point3::new(30.0, 30.0, 5.0),
    );
    // Tilt the box's top about the y axis through (0, 0, 5): slope 1/4.
    let tilt =
        Transform::rotation_about(Point3::new(0.0, 0.0, 5.0), Vec3::Y, -(0.25f64).atan()).unwrap();
    let tool = tool.transformed(&tilt).unwrap();
    check(&intersect(&p, &tool).unwrap(), 250.0 * PI);
}

#[test]
fn a_plane_skimming_an_ellipse_cuts_off_the_thin_sliver_exactly() {
    // The plane y = -b + δ (δ = 1e-4) crosses the ellipse twice, 2 a √(2δ/b) ≈ 0.13
    // apart: two crossings closer than the subdivision's small size, which must both be
    // found. The ellipse is the circle of radius b stretched by a/b in x, so the cut-off
    // segment has (a/b) times the circular segment's area:
    //   s = (a/b) (b² acos((b - δ)/b) - (b - δ) √(2bδ - δ²)),
    // and the sliver of the plate is 10 s.
    let (a, b, d) = (10.0f64, 5.0f64, 1e-4f64);
    let seg = (a / b) * (b * b * ((b - d) / b).acos() - (b - d) * (2.0 * b * d - d * d).sqrt());
    let p = plate();
    let tool = block(
        Point3::new(-20.0, -b + d, -1.0),
        Point3::new(20.0, 20.0, 11.0),
    );
    check(&intersect(&p, &tool).unwrap(), 500.0 * PI - 10.0 * seg);
    let sliver = subtract(&p, &tool).unwrap();
    sliver.validate().unwrap();
    assert!(close(sliver.volume().unwrap(), 10.0 * seg, 1e-6));
}

#[test]
fn spline_sides_touching_a_plane_along_a_ruling_stay_separate_lumps() {
    // An ellipse a = 4, b = 1 about (3, 0) reaches x = -1 at the end of its major axis,
    // where its two halves meet; a box standing on x <= -1 touches it along that ruling.
    // The union is both solids, touching: π · 4 · 1 · 4 + 11 · 8 · 6 = 16π + 528.
    let e = extrude(
        &Frame::XY.with_origin(Point3::new(3.0, 0.0, 2.0)).unwrap(),
        &ellipse(Point2::ORIGIN, 4.0, 1.0, 0.0),
        4.0,
    )
    .unwrap();
    let b = block(Point3::new(-12.0, -2.0, 0.0), Point3::new(-1.0, 6.0, 6.0));
    let u = union(&e, &b).unwrap();
    check(&u, 16.0 * PI + 528.0);
    assert!(matches!(intersect(&e, &b), Err(KernelError::EmptyResult)));
    // The arch's apex (10, 10), inside its spline face, touches a box lying on y >= 10:
    // 5 · 400/3 + 12 · 2 · 3.
    let arch5 = extrude(&Frame::XY, &arch(), 5.0).unwrap();
    let lid = block(Point3::new(4.0, 10.0, 1.0), Point3::new(16.0, 12.0, 4.0));
    check(&union(&arch5, &lid).unwrap(), 2000.0 / 3.0 + 72.0);
    check(&subtract(&arch5, &lid).unwrap(), 2000.0 / 3.0);
}

#[test]
fn unsupported_pairs_are_refused_and_name_the_faces() {
    // A sphere reaching the elliptic side meets it in a space curve: refused.
    let p = plate();
    let ball = crate::revolve(
        &Frame::XZ,
        &Profile::new(
            vec![
                ProfileEdge::Arc {
                    centre: Point2::ORIGIN,
                    radius: 3.0,
                    start_angle: -FRAC_PI_2,
                    sweep: PI,
                },
                ProfileEdge::line(pt(0.0, 3.0), pt(0.0, -3.0)),
            ],
            vec![],
        ),
        &crate::Line2::new(Point2::ORIGIN, chamfera_geom::Vec2::Y).unwrap(),
        2.0 * PI,
    )
    .unwrap()
    .transformed(&Transform::translation(Vec3::new(10.0, 0.0, 5.0)).unwrap())
    .unwrap();
    match union(&p, &ball) {
        Err(KernelError::Unsupported(m)) => assert!(m.contains("face"), "{m}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    // A cylinder across the extrusion direction (along x) through the plate: refused.
    let bar = extrude(
        &Frame::YZ.with_origin(Point3::new(-20.0, 0.0, 0.0)).unwrap(),
        &Profile::circle(pt(0.0, 5.0), 1.0),
        40.0,
    )
    .unwrap();
    assert!(matches!(
        subtract(&p, &bar),
        Err(KernelError::Unsupported(_))
    ));
}

/// A xorshift generator (deterministic, so a failure reproduces).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// An integer in `lo..=hi` (on the grid) or a real in `[lo, hi]`.
    fn draw(&mut self, lo: i64, hi: i64, grid: bool) -> f64 {
        if grid {
            (lo + (self.next() % (hi - lo + 1) as u64) as i64) as f64
        } else {
            lo as f64 + (hi - lo) as f64 * self.unit()
        }
    }
}

/// A random solid with its exact volume: a box, a vertical cylinder, an ellipse plate or
/// the arch, at integer positions (so caps coincide, sides touch and edges meet) or
/// continuous ones.
fn random_solid(rng: &mut Rng, grid: bool) -> (Body, f64) {
    let z0 = rng.draw(-2, 6, grid);
    let h = rng.draw(2, 8, grid);
    let kind = rng.next() % 4;
    let at = |x: f64, y: f64| Frame::XY.with_origin(Point3::new(x, y, z0)).unwrap();
    match kind {
        0 => {
            let (x, y) = (rng.draw(-12, 8, grid), rng.draw(-8, 6, grid));
            let (w, d) = (rng.draw(2, 12, grid), rng.draw(2, 10, grid));
            (
                block(Point3::new(x, y, z0), Point3::new(x + w, y + d, z0 + h)),
                w * d * h,
            )
        }
        1 => {
            let (x, y, r) = (
                rng.draw(-10, 10, grid),
                rng.draw(-6, 6, grid),
                rng.draw(1, 5, grid),
            );
            (
                extrude(&at(x, y), &Profile::circle(Point2::ORIGIN, r), h).unwrap(),
                PI * r * r * h,
            )
        }
        2 => {
            let (x, y) = (rng.draw(-6, 6, grid), rng.draw(-4, 4, grid));
            let (a, b) = (rng.draw(3, 10, grid), rng.draw(1, 3, grid));
            let angle = if grid {
                FRAC_PI_2 * rng.draw(0, 3, true)
            } else {
                PI * rng.unit()
            };
            (
                extrude(&at(x, y), &ellipse(Point2::ORIGIN, a, b, angle), h).unwrap(),
                PI * a * b * h,
            )
        }
        _ => {
            let (x, y) = (rng.draw(-15, 0, grid), rng.draw(-8, 0, grid));
            (extrude(&at(x, y), &arch(), h).unwrap(), 400.0 / 3.0 * h)
        }
    }
}

fn volume_of(r: crate::KernelResult<Body>) -> crate::KernelResult<f64> {
    match r {
        Ok(b) => {
            b.validate()?;
            b.volume()
        }
        Err(KernelError::EmptyResult) => Ok(0.0),
        Err(e) => Err(e),
    }
}

#[test]
fn random_spline_solids_satisfy_the_volume_identities() {
    // V(A ∪ B) + V(A ∩ B) = V(A) + V(B) and V(A - B) = V(A) - V(A ∩ B) to 1e-9 of
    // V(A) + V(B). Every solid extrudes along z, so every pair is a supported one.
    for (seed, grid) in [
        (0x2545_f491_4f6c_dd1d_u64, true),
        (0x9e37_79b9_7f4a_7c15, false),
    ] {
        let mut rng = Rng(seed);
        let mut failed = Vec::new();
        // More pairs on request (a longer sweep, e.g. in release builds).
        let n = std::env::var("CHAMFERA_SPLINE_PAIRS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(12);
        for pair in 0..n {
            let (a, va) = random_solid(&mut rng, grid);
            let (b, vb) = random_solid(&mut rng, grid);
            let results = [
                volume_of(union(&a, &b)),
                volume_of(intersect(&a, &b)),
                volume_of(subtract(&a, &b)),
            ];
            for r in &results {
                if let Err(e) = r {
                    failed.push(format!(
                        "pair {pair}: {e}: A {:?} B {:?}",
                        a.bounding_box(),
                        b.bounding_box()
                    ));
                }
            }
            let tol = 1e-9 * (va + vb);
            if let [Ok(u), Ok(i), _] = &results {
                assert!(
                    (u + i - va - vb).abs() <= tol,
                    "union + intersection: {u} + {i} vs {va} + {vb}"
                );
            }
            if let [_, Ok(i), Ok(d)] = &results {
                assert!((d - (va - i)).abs() <= tol, "difference: {d} vs {va} - {i}");
            }
        }
        println!("grid {grid}: {} of {} refused", failed.len(), 3 * n);
        for f in &failed {
            println!("{f}");
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }
}
