//! Toolpaths on a worker thread (desktop only): after a rebuild the session leaves the
//! toolpaths it could not take from its cache as a job (`Session::take_cam_job`); the
//! worker makes them with the same pure function the session uses (`CamJob::run`) and
//! sends them back, and `Session::accept_cam_result` shows them. Until then the old
//! toolpaths show dimmed with a "Regenerating toolpaths…" badge over the viewport. A
//! newer job cancels the one running (`chamfera_cam::cancel`), and the worker skips
//! queued jobs older than the newest. Commands that read toolpaths never wait for it:
//! they make the stale ones themselves.

use crate::ChamferaApp;
use crate::widgets;
use chamfera_engine::cam_cache::{CamJob, CamJobResult};
use egui::{CornerRadius, FontId, Rect, Stroke, vec2};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};

/// The worker thread's ends.
pub struct CamWorker {
    jobs: Sender<CamJob>,
    results: Receiver<CamJobResult>,
    /// The newest generation wanted: older jobs stop.
    latest: Arc<AtomicU64>,
}

impl CamWorker {
    /// Starts the worker, or `None` when the platform has no threads.
    pub fn spawn() -> Option<CamWorker> {
        let (jobs, rx) = std::sync::mpsc::channel::<CamJob>();
        let (tx, results) = std::sync::mpsc::channel::<CamJobResult>();
        let latest = Arc::new(AtomicU64::new(0));
        let mine = latest.clone();
        std::thread::Builder::new()
            .name("toolpaths".into())
            .spawn(move || {
                while let Ok(mut job) = rx.recv() {
                    // Only the newest queued job is wanted.
                    while let Ok(newer) = rx.try_recv() {
                        job = newer;
                    }
                    let result = job.run_unless_newer(mine.clone());
                    if tx.send(result).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(CamWorker {
            jobs,
            results,
            latest,
        })
    }
}

impl ChamferaApp {
    /// Hands the session's toolpath job to the worker and takes back what it made.
    pub(crate) fn cam_poll(&mut self, ctx: &egui::Context) {
        let Some(w) = self.cam_worker.as_ref() else {
            return;
        };
        w.latest
            .store(self.session.cam_generation(), Ordering::SeqCst);
        if let Some(job) = self.session.take_cam_job()
            && w.jobs.send(job).is_err()
        {
            // The worker is gone: make them here instead.
            self.cam_worker = None;
            self.session.set_cam_background(false);
            return;
        }
        let mut got = Vec::new();
        while let Ok(r) = w.results.try_recv() {
            got.push(r);
        }
        for r in got {
            self.session.accept_cam_result(r);
        }
        if self.session.cam_regenerating() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    /// The "Regenerating toolpaths…" badge over the viewport while the worker runs.
    pub(crate) fn cam_badge(&self, ui: &mut egui::Ui, rect: Rect) {
        if !(self.cam.active && self.session.cam_regenerating()) {
            return;
        }
        let look = self.look();
        let text = "Regenerating toolpaths…";
        let font = FontId::proportional(widgets::T_SMALL);
        let width = ui
            .painter()
            .layout_no_wrap(text.to_owned(), font.clone(), look.c(|t| t.text))
            .size()
            .x
            + 24.0;
        let badge = Rect::from_center_size(
            egui::pos2(rect.center().x, rect.min.y + 22.0),
            vec2(width, 24.0),
        );
        ui.painter().rect(
            badge,
            CornerRadius::same(widgets::R_LARGE),
            look.c(|t| t.panel).gamma_multiply(0.94),
            Stroke::new(1.0, look.c(|t| t.border)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            text,
            font,
            look.c(|t| t.text_muted),
        );
    }
}
