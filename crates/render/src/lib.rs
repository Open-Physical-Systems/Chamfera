//! Layer 1: camera math and the headless scene description.
//!
//! No GPU code lives here: the scene is plain data that the UI layer draws, and that
//! `chamfera-cli snapshot` (M1) renders without a window.

pub use chamfera_geom as geom;
