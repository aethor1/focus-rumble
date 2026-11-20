//! State shared between the focus, audio and UI threads.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

#[derive(Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Relaxed))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Relaxed)
    }
}

#[derive(Default)]
pub struct Shared {
    pub enabled: AtomicBool,
    /// Foreground process to capture (0 = none yet).
    pub target_pid: AtomicU32,
    pub target_name: Mutex<String>,
    pub capture_state: Mutex<String>,
    pub cable_found: AtomicBool,
    pub cable_status: Mutex<String>,
    pub rms_db: AtomicF32,
    pub peak_db: AtomicF32,
    pub latency_ms: AtomicF32,
    pub underruns: AtomicU64,
    /// Launcher-mode messages (profile patching, child process).
    pub launcher_note: Mutex<String>,
}

pub fn set(m: &Mutex<String>, s: impl Into<String>) {
    *m.lock().unwrap() = s.into();
}

pub fn get(m: &Mutex<String>) -> String {
    m.lock().unwrap().clone()
}
