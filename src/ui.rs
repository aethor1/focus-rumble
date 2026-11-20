use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

use eframe::egui::{self, Color32, ProgressBar, RichText};

use crate::shared::{self, Shared};

const VB_CABLE_URL: &str = "https://vb-audio.com/Cable/";

pub struct App {
    pub shared: Arc<Shared>,
}

fn meter_fraction(db: f32) -> f32 {
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let s = &self.shared;
        egui::CentralPanel::default().show(ui, |ui| {
            let mut enabled = s.enabled.load(Relaxed);
            if ui
                .checkbox(&mut enabled, "Route focused app to CABLE Input")
                .changed()
            {
                s.enabled.store(enabled, Relaxed);
            }
            ui.separator();

            egui::Grid::new("stats")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    let pid = s.target_pid.load(Relaxed);
                    ui.label("Focused app");
                    ui.label(if pid == 0 {
                        "-".to_string()
                    } else {
                        format!("{} ({pid})", shared::get(&s.target_name))
                    });
                    ui.end_row();

                    ui.label("Capture");
                    ui.label(shared::get(&s.capture_state));
                    ui.end_row();

                    let (rms, peak) = (s.rms_db.get(), s.peak_db.get());
                    ui.label("Level");
                    ui.vertical(|ui| {
                        ui.add(ProgressBar::new(meter_fraction(rms)).desired_width(220.0));
                        ui.add(ProgressBar::new(meter_fraction(peak)).desired_width(220.0));
                        ui.label(format!("RMS {rms:.1} dBFS / peak {peak:.1} dBFS"));
                    });
                    ui.end_row();

                    ui.label("Added latency");
                    ui.label(format!("~{:.1} ms", s.latency_ms.get()));
                    ui.end_row();

                    ui.label("Underruns");
                    ui.label(s.underruns.load(Relaxed).to_string());
                    ui.end_row();

                    ui.label("Output");
                    ui.label(shared::get(&s.cable_status));
                    ui.end_row();
                });

            // Status stays empty until the render thread has looked for the device.
            if !s.cable_found.load(Relaxed) && !shared::get(&s.cable_status).is_empty() {
                ui.separator();
                ui.label(
                    RichText::new(
                        "CABLE Input not found. Install VB-Cable and reboot; focus-rumble picks it up automatically.",
                    )
                    .color(Color32::from_rgb(230, 120, 60)),
                );
                ui.hyperlink(VB_CABLE_URL);
            }

            let note = shared::get(&s.launcher_note);
            if !note.is_empty() {
                ui.separator();
                ui.label(note);
            }
        });

        // ~30 fps while visible; eframe runs no UI pass at all while minimized.
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
}
