//! Exact measures of planar faces and solids bounded by planar faces.
//!
//! Face area: the vector area of a loop is `½ ∮ (p - r) × dp` (Stokes' theorem on the
//! plane), summed in closed form per edge ([`Curve::area_moment`](super::Curve::area_moment)). With outer loops
//! counter-clockwise and inner loops clockwise about the outward normal `n`, the face
//! area is `n · Σ_loops A_loop`: holes subtract themselves.
//!
//! Volume: by the divergence theorem with the field `F(p) = (p - c) / 3` (divergence 1),
//! `V = ∮ F · n dA = ⅓ Σ_faces ((o_f - c) · n_f) A_f` for planar faces, because
//! `(p - c) · n_f` is constant on the plane through `o_f`. `c` is any point; a point on
//! the body keeps the sum accurate.
//!
//! Every face is planar until K3 adds curved surfaces; these measures need a surface on
//! every face and a curve on every edge.

use super::{Body, FaceId, ShellId, Surface};
use crate::{KernelError, KernelResult};
use chamfera_geom::{Point3, Vec3};

impl Body {
    /// Outward unit normal and a point of a planar face.
    fn planar_face(&self, face: FaceId) -> KernelResult<(Vec3, Point3)> {
        match self.face_surface(face)? {
            Some((Surface::Plane(frame), same_sense)) => {
                let n = if same_sense { frame.z() } else { -frame.z() };
                Ok((n, frame.origin()))
            }
            None => Err(KernelError::InvalidInput(format!(
                "{face} has no surface yet"
            ))),
        }
    }

    /// The area of a planar face (outer loop minus holes), exact for faces bounded by
    /// the K1 curves.
    pub fn face_area(&self, face: FaceId) -> KernelResult<f64> {
        let (n, _) = self.planar_face(face)?;
        let mut sum = Vec3::ZERO;
        for l in self.face_loops(face)? {
            sum = sum + self.loop_vector_area(l)?;
        }
        Ok(sum.dot(n))
    }

    /// The total area of every face of the body.
    pub fn area(&self) -> KernelResult<f64> {
        self.faces()
            .into_iter()
            .map(|f| self.face_area(f))
            .sum::<KernelResult<f64>>()
    }

    /// The volume enclosed by one closed shell of planar faces.
    pub fn shell_volume(&self, shell: ShellId) -> KernelResult<f64> {
        let faces = self.shell_faces(shell)?;
        let Some(first) = faces.first() else {
            return Ok(0.0);
        };
        let c = match self.face_loops(*first)?.first() {
            Some(l) => match self.loop_vertices(*l)?.first() {
                Some(v) => self.vertex_point(*v)?,
                None => Point3::ORIGIN,
            },
            None => Point3::ORIGIN,
        };
        let mut v = 0.0;
        for f in faces {
            let (n, o) = self.planar_face(f)?;
            v += (o - c).dot(n) * self.face_area(f)?;
        }
        Ok(v / 3.0)
    }

    /// The volume of the body: the sum over its shells.
    pub fn volume(&self) -> KernelResult<f64> {
        self.shells()
            .into_iter()
            .map(|s| self.shell_volume(s))
            .sum::<KernelResult<f64>>()
    }
}
