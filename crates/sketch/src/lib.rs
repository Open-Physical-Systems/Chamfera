//! Layer 1: 2D sketch entities and Chamfera's own constraint solver.
//!
//! Empty in M0. M2 adds points, lines, arcs, circles and splines; geometric constraints and
//! dimensions; a Newton / Levenberg-Marquardt solver with degree-of-freedom analysis; and
//! closed-profile detection. Read `skills/chamfera-kernel/SKILL.md` for the clean-room rules
//! that apply to solver work too.

pub use chamfera_geom as geom;
