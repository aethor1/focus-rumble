//! Platform-independent audio helpers: level metering and the jitter FIFO
//! between the capture and render streams.

use std::collections::VecDeque;

/// Interleaved stereo everywhere.
pub const CHANNELS: usize = 2;
/// Floor used for silence so the UI never shows -inf.
pub const DB_FLOOR: f32 = -100.0;

pub fn to_dbfs(amplitude: f32) -> f32 {
    if amplitude <= 1e-5 {
        DB_FLOOR
    } else {
        (20.0 * amplitude.log10()).max(DB_FLOOR)
    }
}

/// Returns (rms dBFS, peak dBFS) over all samples.
pub fn meter(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (DB_FLOOR, DB_FLOOR);
    }
    let mut sum = 0.0f64;
    let mut peak = 0.0f32;
    for &s in samples {
        sum += f64::from(s) * f64::from(s);
        peak = peak.max(s.abs());
    }
    let rms = (sum / samples.len() as f64).sqrt() as f32;
    (to_dbfs(rms), to_dbfs(peak))
}

/// Jitter buffer: capture pushes, render pops. Bounded so added latency stays low.
pub struct Fifo {
    buf: VecDeque<f32>,
    max_frames: usize,
    prefill_frames: usize,
    fade_frames: usize,
    fade_in_left: usize,
    primed: bool,
    pub underruns: u64,
}

impl Fifo {
    pub fn new(max_frames: usize, prefill_frames: usize, fade_frames: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(max_frames * CHANNELS * 2),
            max_frames,
            prefill_frames,
            fade_frames,
            fade_in_left: 0,
            primed: false,
            underruns: 0,
        }
    }

    pub fn frames(&self) -> usize {
        self.buf.len() / CHANNELS
    }

    /// Append interleaved samples, applying any pending fade-in, and drop the
    /// oldest frames if the queue grew past `max_frames` (clock drift).
    pub fn push(&mut self, data: &[f32]) {
        for frame in data.as_chunks::<CHANNELS>().0 {
            let gain = if self.fade_in_left > 0 {
                let g = 1.0 - self.fade_in_left as f32 / self.fade_frames as f32;
                self.fade_in_left -= 1;
                g
            } else {
                1.0
            };
            self.buf.extend(frame.iter().map(|s| s * gain));
        }
        let excess = self.frames().saturating_sub(self.max_frames);
        self.buf.drain(..excess * CHANNELS);
    }

    /// Fill `out` completely. Outputs silence until `prefill_frames` are queued;
    /// running dry while playing counts as one underrun and re-primes.
    pub fn pop(&mut self, out: &mut [f32]) {
        let need = out.len() / CHANNELS;
        if !self.primed && self.frames() >= self.prefill_frames.max(1) {
            self.primed = true;
        }
        if !self.primed {
            out.fill(0.0);
            return;
        }
        let take = self.frames().min(need) * CHANNELS;
        for (o, s) in out.iter_mut().zip(self.buf.drain(..take)) {
            *o = s;
        }
        if take < out.len() {
            out[take..].fill(0.0);
            self.underruns += 1;
            self.primed = false;
        }
    }

    /// Called on a source switch: ramp what is still queued down to zero so the
    /// old stream ends without a click, and fade the next stream in.
    pub fn fade_out_and_restart(&mut self) {
        let frames = self.frames();
        for (i, frame) in self
            .buf
            .make_contiguous()
            .as_chunks_mut::<CHANNELS>()
            .0
            .iter_mut()
            .enumerate()
        {
            let gain = 1.0 - (i + 1) as f32 / frames as f32;
            frame.iter_mut().for_each(|s| *s *= gain);
        }
        self.fade_in_left = self.fade_frames;
    }

    /// Drop everything without counting an underrun (capture stopped).
    pub fn clear(&mut self) {
        self.buf.clear();
        self.primed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn meter_full_scale_sine() {
        let s: Vec<f32> = (0..4800)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI / 48.0).sin())
            .collect();
        let (rms, peak) = meter(&s);
        assert!(close(rms, -3.01), "rms {rms}");
        assert!(close(peak, 0.0), "peak {peak}");
    }

    #[test]
    fn meter_square_and_silence() {
        let (rms, peak) = meter(&[0.5, -0.5, 0.5, -0.5]);
        assert!(close(rms, -6.02) && close(peak, -6.02));
        assert_eq!(meter(&[0.0; 8]), (DB_FLOOR, DB_FLOOR));
        assert_eq!(meter(&[]), (DB_FLOOR, DB_FLOOR));
        assert!(close(to_dbfs(0.1), -20.0));
    }

    #[test]
    fn fifo_prefill_underrun_and_cap() {
        let mut f = Fifo::new(8, 3, 0);
        let mut out = [9.0f32; 4]; // 2 frames
        f.push(&[1.0; 4]); // 2 frames < prefill 3 -> silence, no underrun
        f.pop(&mut out);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(f.underruns, 0);

        f.push(&[1.0; 2]); // 3 frames -> primed
        f.pop(&mut out);
        assert_eq!(out, [1.0; 4]);
        assert_eq!(f.frames(), 1);

        f.pop(&mut out); // 1 of 2 frames -> underrun, re-prime
        assert_eq!(out, [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(f.underruns, 1);
        f.push(&[1.0; 2]);
        f.pop(&mut out); // 1 frame < prefill -> silence, not another underrun
        assert_eq!(out, [0.0; 4]);
        assert_eq!(f.underruns, 1);

        f.push(&[2.0; 40]); // 21 frames, capped to newest 8
        assert_eq!(f.frames(), 8);
    }

    #[test]
    fn fifo_fades() {
        let mut f = Fifo::new(100, 1, 4);
        f.push(&[1.0; 8]); // 4 frames, no fade yet
        f.fade_out_and_restart();
        let mut out = [0.0f32; 8];
        f.pop(&mut out);
        assert_eq!(out, [0.75, 0.75, 0.5, 0.5, 0.25, 0.25, 0.0, 0.0]);
        f.push(&[1.0; 10]); // fade in over 4 frames
        let mut out = [0.0f32; 10];
        f.pop(&mut out);
        assert_eq!(out, [0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0]);
    }
}
