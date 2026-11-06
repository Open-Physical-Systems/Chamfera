//! Input validation shared by the curve constructors. Every message names the value and
//! says what is allowed.

use crate::{KernelError, KernelResult};
use chamfera_geom::{
    ANGULAR_TOLERANCE, LINEAR_TOLERANCE, MAX_COORDINATE, Point2, Point3, Vec2, Vec3,
};
use std::f64::consts::TAU;

pub(crate) fn point3(name: &str, p: Point3) -> KernelResult<Point3> {
    if p.is_valid() {
        Ok(p)
    } else {
        Err(KernelError::InvalidInput(format!(
            "`{name}` must have finite coordinates within ±{MAX_COORDINATE} mm"
        )))
    }
}

pub(crate) fn point2(name: &str, p: Point2) -> KernelResult<Point2> {
    if p.is_valid() {
        Ok(p)
    } else {
        Err(KernelError::InvalidInput(format!(
            "`{name}` must have finite coordinates within ±{MAX_COORDINATE} mm"
        )))
    }
}

pub(crate) fn direction3(name: &str, v: Vec3) -> KernelResult<Vec3> {
    v.normalized().ok_or_else(|| {
        KernelError::InvalidInput(format!(
            "`{name}` must be a finite vector longer than {LINEAR_TOLERANCE} mm"
        ))
    })
}

pub(crate) fn direction2(name: &str, v: Vec2) -> KernelResult<Vec2> {
    v.normalized().ok_or_else(|| {
        KernelError::InvalidInput(format!(
            "`{name}` must be a finite vector longer than {LINEAR_TOLERANCE} mm"
        ))
    })
}

/// A radius or semi-axis: finite, longer than the linear tolerance, within the model range.
pub(crate) fn length(name: &str, v: f64) -> KernelResult<f64> {
    if v.is_finite() && v > LINEAR_TOLERANCE && v <= MAX_COORDINATE {
        Ok(v)
    } else {
        Err(KernelError::InvalidInput(format!(
            "`{name}` must be a finite length between {LINEAR_TOLERANCE} and {MAX_COORDINATE} mm"
        )))
    }
}

/// A start angle: any finite value, normalised into `[0, 2π)`.
pub(crate) fn start_angle(name: &str, v: f64) -> KernelResult<f64> {
    if v.is_finite() {
        Ok(v.rem_euclid(TAU))
    } else {
        Err(KernelError::InvalidInput(format!(
            "`{name}` must be a finite angle in radians"
        )))
    }
}

/// A sweep angle: finite, non-zero, at most one full turn; returned with its sign and a
/// magnitude clamped to 2π. The arc it makes with `radius` must be longer than the linear
/// tolerance.
pub(crate) fn sweep(name: &str, v: f64, radius: f64) -> KernelResult<f64> {
    let ok = v.is_finite()
        && v.abs() > ANGULAR_TOLERANCE
        && v.abs() <= TAU + ANGULAR_TOLERANCE
        && v.abs() * radius > LINEAR_TOLERANCE;
    if ok {
        Ok(v.signum() * v.abs().min(TAU))
    } else {
        Err(KernelError::InvalidInput(format!(
            "`{name}` must be a finite angle with 0 < |{name}| <= 2π radians, giving an arc longer than {LINEAR_TOLERANCE} mm"
        )))
    }
}

/// `theta` moved by whole turns to the first value `>= lo`.
pub(crate) fn angle_from(theta: f64, lo: f64) -> f64 {
    lo + (theta - lo).rem_euclid(TAU)
}

/// `theta` as a parameter of the angular range `[lo, hi]` (`hi - lo <= 2π`), accepting
/// `slack` radians past either end (and clamping onto the range), or `None` outside.
pub(crate) fn angle_in_range(theta: f64, lo: f64, hi: f64, slack: f64) -> Option<f64> {
    let a = angle_from(theta, lo);
    if a <= hi {
        Some(a)
    } else if a <= hi + slack {
        Some(hi)
    } else if a >= lo + TAU - slack {
        Some(lo)
    } else {
        None
    }
}
