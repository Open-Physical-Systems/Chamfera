//! Holding tabs on a contour: bridges of material left standing at the bottom of the cut,
//! so a part cut out of a plate stays attached to it.
//!
//! The tool centre runs along a closed path; along it (arc length `s`) the tabs sit at
//! centres `c`. Over a tab the path's height is the tab's profile seen through the tool:
//! a flat end mill of radius `r` whose centre is at `s` cuts the material along the edge
//! from `s - r` to `s + r`, so to leave a tab of profile `T(x)` the centre may go no lower
//! than the highest `T` within `r` of it. For a flat-topped tab `w` wide that is the tab
//! height over `|s - c| <= w / 2 + r`; for a triangle `w` wide at its foot and `h` tall it
//! is `h` over `|s - c| <= r`, falling at `h / (w / 2)` per mm beyond. Flat-topped tabs
//! are climbed onto and left along ramps at the tabs' ramp angle, outside the flat part
//! (a right angle goes straight up and down). The profile is exact in `s`; the path is
//! cut at every place it changes slope, so each piece is a line or an arc (a helix where
//! it climbs) with linear height.
//!
//! Tabs are spread evenly along the path, then each one is slid to the nearest place
//! where its whole footprint is on one straight piece of the path, where there is one
//! (a tab on a corner arc holds poorly and is hard to clean up).

use crate::CamError;
use crate::path2::{JOIN_TOLERANCE, Seg, loop_length};
use crate::toolpath::Move;
use chamfera_doc::cam::{MAX_TABS, TabShape, Tabs};
use chamfera_geom::Point3;

/// Where the tabs of one closed path are, and their profile.
#[derive(Debug, Clone, PartialEq)]
pub struct TabPlan {
    /// Tab centres, as distances along the path from its start, ascending.
    pub centres: Vec<f64>,
    /// Half the length of the flat top of the path over a tab (mm).
    pub plateau: f64,
    /// How fast the path falls beyond the flat top (height per length); `None` straight
    /// down.
    pub slope: Option<f64>,
    /// The height of the tabs' tops (absolute).
    pub top: f64,
    /// The bottom of the cut (absolute), where the tabs stand.
    pub bottom: f64,
}

impl TabPlan {
    /// How far from a tab's centre its footprint reaches on a pass at `z` (mm).
    fn reach(&self, z: f64) -> f64 {
        let h = (self.top - z).max(0.0);
        self.plateau + self.slope.map_or(0.0, |m| h / m)
    }

    /// The tab profile's height at distance `d` from a tab centre (`-inf` beyond it when
    /// the sides are vertical).
    fn profile(&self, d: f64) -> f64 {
        if d <= self.plateau {
            self.top
        } else {
            match self.slope {
                Some(m) => self.top - (d - self.plateau) * m,
                None => f64::NEG_INFINITY,
            }
        }
    }

    /// The path's height at `s` on a pass at `z`, worked out as the linear piece active
    /// over `[a, b]` (found at its middle) evaluated at `s`.
    fn z_on(&self, a: f64, b: f64, s: f64, z: f64) -> f64 {
        let m = 0.5 * (a + b);
        let mut best: Option<(f64, f64)> = None;
        for &c in &self.centres {
            let v = self.profile((m - c).abs());
            if v > z && best.is_none_or(|(bv, _)| v > bv) {
                best = Some((v, c));
            }
        }
        match best {
            None => z,
            Some((_, c)) => self.profile((s - c).abs()).max(z),
        }
    }
}

/// Where `tabs` go on the closed path `l` (the tool centre's), for a tool of radius
/// `tool_radius` cutting from `stock_top` down to `bottom`. Tabs taller than the cut
/// stop at the stock top.
pub fn plan(
    l: &[Seg],
    tabs: &Tabs,
    tool_radius: f64,
    bottom: f64,
    stock_top: f64,
) -> Result<TabPlan, CamError> {
    let max = chamfera_doc::cam::MAX_TOOL_SIZE;
    let ok = |v: f64| v.is_finite() && v > 0.0 && v <= max;
    if !(ok(tabs.width) && ok(tabs.height) && tool_radius.is_finite() && tool_radius >= 0.0) {
        return Err(CamError::Invalid(format!(
            "tabs need a width and height above 0 and at most {max} mm"
        )));
    }
    let total = loop_length(l);
    if !(total.is_finite() && total > JOIN_TOLERANCE) {
        return Err(CamError::Invalid("the path has no length".into()));
    }
    let (plateau, slope) = match tabs.shape {
        TabShape::Rectangular => {
            let a = tabs.ramp_angle;
            if !(a.is_finite() && a > 0.0 && a <= std::f64::consts::FRAC_PI_2 + 1e-12) {
                return Err(CamError::Invalid(
                    "the tab ramp angle must be above 0 and at most 90 degrees".into(),
                ));
            }
            let slope = if a >= std::f64::consts::FRAC_PI_2 - 1e-9 {
                None
            } else {
                Some(a.tan())
            };
            (tabs.width / 2.0 + tool_radius, slope)
        }
        TabShape::Triangular => (tool_radius, Some(tabs.height / (tabs.width / 2.0))),
    };
    let n = match (tabs.count, tabs.spacing) {
        (Some(n), None) if (1..=MAX_TABS).contains(&n) => n,
        (None, Some(sp)) if sp.is_finite() && sp > 0.0 && sp <= chamfera_geom::MAX_COORDINATE => {
            let n = (total / sp - 1e-9).ceil().max(1.0);
            if n > f64::from(MAX_TABS) {
                return Err(CamError::TooComplex(format!(
                    "a tab spacing of {sp} mm makes more than {MAX_TABS} tabs"
                )));
            }
            n as u32
        }
        _ => {
            return Err(CamError::Invalid(
                "tabs need either a count from 1 or a positive spacing".into(),
            ));
        }
    };
    let mut p = TabPlan {
        centres: Vec::new(),
        plateau,
        slope,
        top: (bottom + tabs.height).min(stock_top.max(bottom)),
        bottom,
    };
    // The footprint at the deepest pass.
    let half = p.reach(bottom);
    if 2.0 * half * f64::from(n) > total + JOIN_TOLERANCE {
        return Err(CamError::Refused(format!(
            "{n} tab{} of this size do not fit along a path {} mm long",
            if n == 1 { "" } else { "s" },
            crate::post::number(total)
        )));
    }
    // Places whose whole footprint is on one straight piece: [from, to] per line.
    let mut windows: Vec<(f64, f64)> = Vec::new();
    let mut s0 = 0.0;
    for seg in l {
        let len = seg.length();
        if matches!(seg, Seg::Line { .. }) && len >= 2.0 * half {
            windows.push((s0 + half, s0 + len - half));
        }
        s0 += len;
    }
    for k in 0..n {
        let even = total * (f64::from(k) + 0.5) / f64::from(n);
        let c = windows
            .iter()
            .map(|&(a, b)| even.max(a).min(b))
            .min_by(|x, y| (x - even).abs().total_cmp(&(y - even).abs()))
            .unwrap_or_else(|| even.max(half).min(total - half));
        p.centres.push(c);
    }
    p.centres.sort_by(f64::total_cmp);
    p.centres.dedup_by(|a, b| (*a - *b).abs() <= JOIN_TOLERANCE);
    Ok(p)
}

/// One pass round the closed path `l` at height `z`, rising over the tabs of `plan`
/// where they stand above `z`: ramps and runs at `feed`, straight descents at
/// `plunge_feed`. Starts at the path's start, already at `z`, and ends there at `z`.
pub fn pass(l: &[Seg], plan: &TabPlan, z: f64, feed: f64, plunge_feed: f64, out: &mut Vec<Move>) {
    let total = loop_length(l);
    let mut cuts: Vec<f64> = Vec::new();
    if plan.top > z + JOIN_TOLERANCE {
        let reach = plan.reach(z);
        for (i, &c) in plan.centres.iter().enumerate() {
            cuts.extend([c - reach, c - plan.plateau, c + plan.plateau, c + reach]);
            if let Some(&next) = plan.centres.get(i + 1) {
                cuts.push(0.5 * (c + next));
            }
        }
    }
    cuts.retain(|s| s.is_finite() && *s > 0.0 && *s < total);
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
    let mut cur_z = z;
    let mut s0 = 0.0;
    let mut k = 0;
    for seg in l {
        let len = seg.length();
        let s1 = s0 + len;
        // The places along this piece where the profile changes.
        let mut at = vec![s0];
        while let Some(&c) = cuts.get(k) {
            if c >= s1 {
                break;
            }
            if c > s0 + 1e-9 {
                at.push(c);
            }
            k += 1;
        }
        at.push(s1);
        for w in at.windows(2) {
            let &[a, b] = w else {
                continue;
            };
            if b - a <= 1e-9 || len <= 0.0 {
                continue;
            }
            let za = plan.z_on(a, b, a, z);
            let zb = plan.z_on(a, b, b, z);
            let (t0, t1) = ((a - s0) / len, ((b - s0) / len).min(1.0));
            if (za - cur_z).abs() > JOIN_TOLERANCE {
                let p = seg.point_at(t0);
                let f = if za > cur_z { feed } else { plunge_feed };
                out.push(Move::Line {
                    to: Point3::new(p.x, p.y, za),
                    feed: f,
                });
            }
            crate::ops::seg_moves(&seg.sub(t0, t1), za, zb, feed, out);
            cur_z = zb;
        }
        s0 = s1;
    }
    if (cur_z - z).abs() > JOIN_TOLERANCE
        && let Some(p) = l.last().map(Seg::end)
    {
        out.push(Move::Line {
            to: Point3::new(p.x, p.y, z),
            feed: plunge_feed,
        });
    }
}
