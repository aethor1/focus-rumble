//! Capture the focused process tree (process loopback) and play it into CABLE Input.
//!
//! Two threads share a small jitter FIFO: capture never blocks render, so the
//! async loopback activation on a focus switch cannot starve the cable.

use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use wasapi::{
    AudioCaptureClient, AudioClient, Device, DeviceEnumerator, Direction, Handle, SampleType,
    StreamMode, WaveFormat, initialize_mta,
};

use crate::dsp::{CHANNELS, DB_FLOOR, Fifo, meter};
use crate::shared::{self, Shared};

pub const CABLE_NAME: &str = "CABLE Input";
// Both streams use this format with AUTOCONVERTPCM, so Windows converts to the
// cable's own mix format (rate/channels/bit depth) for us.
const RATE: usize = 48_000;
const MS: usize = RATE / 1000;
const FADE_FRAMES: usize = 5 * MS;
const PREFILL_FRAMES: usize = 5 * MS;
const MAX_QUEUE_FRAMES: usize = 20 * MS;

fn format() -> WaveFormat {
    WaveFormat::new(32, 32, SampleType::Float, RATE, CHANNELS, None)
}

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// Render endpoint whose friendly name contains "CABLE Input". Needs COM on this thread.
pub fn find_cable() -> Res<Option<Device>> {
    let devices = DeviceEnumerator::new()?.get_device_collection(Direction::Render)?;
    for dev in &devices {
        let dev = dev?;
        if dev.get_friendlyname()?.contains(CABLE_NAME) {
            return Ok(Some(dev));
        }
    }
    Ok(None)
}

/// Endpoint id (`{0.0.0.00000000}.{guid}`) of CABLE Input, as DSX stores it.
pub fn cable_endpoint_id() -> Res<Option<String>> {
    let _ = initialize_mta();
    Ok(find_cable()?.map(|d| d.get_id()).transpose()?)
}

pub fn spawn(shared: Arc<Shared>) {
    let fifo = Arc::new(Mutex::new(Fifo::new(
        MAX_QUEUE_FRAMES,
        PREFILL_FRAMES,
        FADE_FRAMES,
    )));
    let (s, f) = (shared.clone(), fifo.clone());
    thread::Builder::new()
        .name("render".into())
        .spawn(move || render_thread(&s, &f))
        .expect("spawn render thread");
    thread::Builder::new()
        .name("capture".into())
        .spawn(move || capture_thread(&shared, &fifo))
        .expect("spawn capture thread");
}

fn render_thread(shared: &Shared, fifo: &Mutex<Fifo>) {
    let _ = initialize_mta();
    loop {
        let err = match find_cable() {
            Ok(Some(dev)) => {
                shared.cable_found.store(true, Relaxed);
                render(shared, fifo, &dev)
                    .err()
                    .map(|e| format!("{CABLE_NAME} error: {e}"))
            }
            Ok(None) => {
                shared.cable_found.store(false, Relaxed);
                Some(format!("{CABLE_NAME} not found (VB-Cable not installed?)"))
            }
            Err(e) => Some(format!("Device enumeration failed: {e}")),
        };
        if let Some(e) = err {
            shared::set(&shared.cable_status, e);
        }
        // Retry: covers VB-Cable being installed or the device coming back.
        thread::sleep(Duration::from_secs(2));
    }
}

fn render(shared: &Shared, fifo: &Mutex<Fifo>, dev: &Device) -> Res<()> {
    let mut client = dev.get_iaudioclient()?;
    let (period_hns, _) = client.get_device_period()?;
    client.initialize_client(
        &format(),
        Direction::Render,
        StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 0,
        },
        None,
    )?;
    let event = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;
    let buffer_frames = client.get_buffer_size()? as usize;
    // Keep only ~1.5 engine periods queued in the device instead of the whole buffer.
    let period_frames = (period_hns as usize * RATE).div_ceil(10_000_000);
    let target_padding = (period_frames * 3 / 2).min(buffer_frames);
    let mut samples = vec![0f32; buffer_frames * CHANNELS];
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    client.start_stream()?;
    shared::set(
        &shared.cable_status,
        format!("{} (playing)", dev.get_friendlyname()?),
    );

    loop {
        if event.wait_for_event(200).is_err() {
            continue;
        }
        let padding = client.get_current_padding()? as usize;
        let frames = target_padding.saturating_sub(padding);
        if frames == 0 {
            continue;
        }
        let out = &mut samples[..frames * CHANNELS];
        let queued = {
            let mut f = fifo.lock().unwrap();
            f.pop(out);
            shared.underruns.store(f.underruns, Relaxed);
            f.frames()
        };
        bytes.clear();
        bytes.extend(out.iter().flat_map(|s| s.to_le_bytes()));
        render.write_to_device(&bytes, None)?;
        let added = (queued + padding + frames) as f32 / MS as f32;
        shared.latency_ms.set(added);
    }
}

struct Capture {
    pid: u32,
    _client: AudioClient,
    capture: AudioCaptureClient,
    event: Handle,
}

fn open_capture(pid: u32) -> Res<Capture> {
    let mut client = AudioClient::new_application_loopback_client(pid, true)?;
    client.initialize_client(
        &format(),
        Direction::Capture,
        StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 0,
        },
        None,
    )?;
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;
    client.start_stream()?;
    Ok(Capture {
        pid,
        _client: client,
        capture,
        event,
    })
}

fn capture_thread(shared: &Shared, fifo: &Mutex<Fifo>) {
    let _ = initialize_mta();
    let mut current: Option<Capture> = None;
    let mut failed_pid = 0;
    let mut bytes = vec![0u8; RATE * CHANNELS * 4]; // 1 s, larger than any packet
    let mut samples: Vec<f32> = Vec::with_capacity(RATE * CHANNELS);

    loop {
        let want = if shared.enabled.load(Relaxed) {
            shared.target_pid.load(Relaxed)
        } else {
            0
        };
        if current.as_ref().map_or(0, |c| c.pid) != want {
            if current.take().is_some() {
                let mut f = fifo.lock().unwrap();
                if want == 0 {
                    f.clear()
                } else {
                    f.fade_out_and_restart()
                }
            }
            shared.rms_db.set(DB_FLOOR);
            shared.peak_db.set(DB_FLOOR);
            let name = shared::get(&shared.target_name);
            if want == 0 {
                failed_pid = 0;
                let idle = if shared.enabled.load(Relaxed) {
                    "Waiting for a focused app"
                } else {
                    "Off"
                };
                shared::set(&shared.capture_state, idle);
            } else if want != failed_pid {
                match open_capture(want) {
                    Ok(c) => {
                        failed_pid = 0;
                        shared::set(&shared.capture_state, format!("Capturing {name} ({want})"));
                        current = Some(c);
                    }
                    Err(e) => {
                        failed_pid = want; // don't retry until focus changes
                        shared::set(
                            &shared.capture_state,
                            format!("Capture of {name} failed: {e}"),
                        );
                    }
                }
            }
        }

        let Some(c) = &current else {
            thread::sleep(Duration::from_millis(20));
            continue;
        };
        // Short timeout so focus changes are picked up even while the target is silent.
        let _ = c.event.wait_for_event(20);
        if let Err(e) = drain(c, fifo, shared, &mut bytes, &mut samples) {
            shared::set(&shared.capture_state, format!("Capture error: {e}"));
            current = None;
            failed_pid = want;
        }
    }
}

fn drain(
    c: &Capture,
    fifo: &Mutex<Fifo>,
    shared: &Shared,
    bytes: &mut [u8],
    samples: &mut Vec<f32>,
) -> Res<()> {
    while c.capture.get_next_packet_size()?.unwrap_or(0) > 0 {
        let (frames, _) = c.capture.read_from_device(bytes)?;
        samples.clear();
        samples.extend(
            bytes[..frames as usize * CHANNELS * 4]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
        let (rms, peak) = meter(samples);
        shared.rms_db.set(rms);
        shared.peak_db.set(peak);
        fifo.lock().unwrap().push(samples);
    }
    Ok(())
}
