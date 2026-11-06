//! A small vector type for the tracer: positions, directions and RGB colours.

use chamfera_geom::{Point3, Vec3};
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

pub const fn v3(x: f64, y: f64, z: f64) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    pub const ZERO: V3 = v3(0.0, 0.0, 0.0);
    pub const ONE: V3 = v3(1.0, 1.0, 1.0);

    pub fn splat(v: f64) -> V3 {
        v3(v, v, v)
    }

    pub fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: V3) -> V3 {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    /// Unit length, or `None` for a zero or non-finite vector.
    pub fn normalized(self) -> Option<V3> {
        let l = self.length();
        (l.is_finite() && l > 1e-300).then(|| self * (1.0 / l))
    }

    pub fn min(self, o: V3) -> V3 {
        v3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    pub fn max(self, o: V3) -> V3 {
        v3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    pub fn max_component(self) -> f64 {
        self.x.max(self.y).max(self.z)
    }

    pub fn axis(self, i: usize) -> f64 {
        match i {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn rgb(c: [f32; 3]) -> V3 {
        v3(f64::from(c[0]), f64::from(c[1]), f64::from(c[2]))
    }

    pub fn to_f32(self) -> [f32; 3] {
        [self.x as f32, self.y as f32, self.z as f32]
    }

    /// Mean of the three components (a colour's grey level).
    pub fn mean(self) -> f64 {
        (self.x + self.y + self.z) / 3.0
    }
}

impl From<Point3> for V3 {
    fn from(p: Point3) -> V3 {
        v3(p.x, p.y, p.z)
    }
}

impl From<Vec3> for V3 {
    fn from(p: Vec3) -> V3 {
        v3(p.x, p.y, p.z)
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl AddAssign for V3 {
    fn add_assign(&mut self, o: V3) {
        *self = *self + o;
    }
}

impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        v3(-self.x, -self.y, -self.z)
    }
}

impl Mul<f64> for V3 {
    type Output = V3;
    fn mul(self, k: f64) -> V3 {
        v3(self.x * k, self.y * k, self.z * k)
    }
}

/// Component by component (colours).
impl Mul for V3 {
    type Output = V3;
    fn mul(self, o: V3) -> V3 {
        v3(self.x * o.x, self.y * o.y, self.z * o.z)
    }
}

impl Div<f64> for V3 {
    type Output = V3;
    fn div(self, k: f64) -> V3 {
        v3(self.x / k, self.y / k, self.z / k)
    }
}

/// An orthonormal frame around a unit normal (Duff et al. 2017, "Building an Orthonormal
/// Basis, Revisited").
pub fn frame(n: V3) -> (V3, V3) {
    let s = if n.z >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    (
        v3(1.0 + s * n.x * n.x * a, s * b, -s * n.x),
        v3(b, s + n.y * n.y * a, -n.y),
    )
}

/// A direction in the frame of `n` from its local coordinates.
pub fn to_world(n: V3, local: V3) -> V3 {
    let (t, b) = frame(n);
    t * local.x + b * local.y + n * local.z
}
