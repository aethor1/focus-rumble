#![cfg_attr(windows, windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code))]

mod dsp;
mod dsx;

#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod focus;
#[cfg(windows)]
mod launcher;
#[cfg(windows)]
mod shared;
#[cfg(windows)]
mod ui;

#[cfg(not(windows))]
fn main() {
    eprintln!("focus-rumble only runs on Windows.");
}

#[cfg(windows)]
fn main() -> eframe::Result {
    use std::sync::Arc;
    use std::sync::atomic::Ordering::Relaxed;

    let shared = Arc::new(shared::Shared::default());
    shared.enabled.store(true, Relaxed);

    let command: Vec<String> = std::env::args().skip(1).collect();
    if !command.is_empty() {
        launcher::run(command, shared.clone());
    }
    focus::spawn(shared.clone());
    audio::spawn(shared.clone());

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([420.0, 300.0])
            .with_min_inner_size([360.0, 240.0]),
        ..Default::default()
    };
    eframe::run_native(
        "focus-rumble",
        options,
        Box::new(|_cc| Ok(Box::new(ui::App { shared }))),
    )
}
