//! Presentation renders: the path tracer (`chamfera-raytrace`) fed with the bodies the
//! viewport shows, their materials (`body.appearance`), the view and the environment
//! (`render.environment`). The commands are in `cmd/render.rs`.

use crate::{CommandError, Session};
use chamfera_raytrace::{Body, Render, Settings};
use chamfera_render::{Environment, Theme};
use std::sync::Arc;

impl Session {
    /// A render of what the viewport shows, ready to step. Hidden bodies are left out;
    /// bodies without a colour take the light theme's body colour, as exports do.
    pub fn prepare_render(&self, settings: Settings) -> Result<Render, CommandError> {
        let theme = Theme::light();
        let bodies: Vec<Body> = self
            .scene
            .bodies
            .iter()
            .filter(|b| b.visible)
            .map(|b| {
                let fallback = if b.surface {
                    theme.surface_colour(b.tint)
                } else {
                    theme.body_colour(b.tint)
                };
                Body {
                    mesh: &b.mesh,
                    material: b.appearance.material(fallback),
                }
            })
            .collect();
        let env = Environment::preset(self.scene.display.environment, &self.camera);
        Render::new(&bodies, &self.camera, env, settings)
            .map_err(|e| CommandError::BadParams(e.to_string()))
    }

    /// Starts a progressive render (`render.start`) and keeps it as the session's
    /// render, replacing (and stopping) any earlier one.
    pub fn start_render(&mut self, settings: Settings) -> Result<Arc<Render>, CommandError> {
        let job = Arc::new(self.prepare_render(settings)?);
        if let Some(old) = self.render_job.replace(job.clone()) {
            old.cancel();
        }
        Ok(job)
    }
}
