# focus-rumble

Routes only the focused app's audio into a virtual cable so DSX audio-to-haptics only rumbles for what you're actually playing.

## What it does

```
focused window ──► its process tree's audio ──► CABLE Input (VB-Cable) ──► DSX Audio-to-Haptics ──► DualSense
        (process loopback copy; your speakers still get the normal mix)
```

- Tracks the foreground window (`SetWinEventHook`, no polling) and captures audio from **only that process and its children** using Windows process loopback.
- Switches capture as soon as focus changes, with a 5 ms fade so it doesn't pop. Clicking the focus-rumble window keeps the previous target.
- Plays the copy into the **CABLE Input** endpoint (event-driven shared mode, aiming for 20 ms or less of added latency). DSX keeps its own haptics engine and just listens to the cable.
- A small window shows the focused app (exe and PID), capture state, an RMS/peak meter in dBFS, the estimated added latency, the underrun count, the cable status and an on/off toggle.
- **Launcher mode** (`focus-rumble.exe <command...>`): if DSX isn't running yet, it finds the DSX install (from the command path or your Steam libraries), backs up every `.dsx` profile under `DSX_Savefile\Configuration Files\{Controller Profiles,Game Profiles}`, and sets `audio_haptics_output_device_id` and `audio_to_rumble_output_device_id` to the CABLE Input endpoint id. Nothing else in the files is changed. It then starts the command and exits when the command exits.
- With no arguments it runs standalone and doesn't touch the profiles.

Backups go to `DSX_Savefile\Configuration Files\focus-rumble-backup\` the first time each profile is changed. Later runs never overwrite them, so they always hold your original settings.

## Setup

1. Install **VB-Cable** from <https://vb-audio.com/Cable/> and reboot. focus-rumble never installs it for you.
2. Download `focus-rumble.exe` from the latest release, or from the `focus-rumble-exe` artifact of a CI run, and put it anywhere.
3. In Steam, right-click **DSX** → **Properties** → **General** → **Launch Options**, and enter:
   ```
   "C:\path\to\focus-rumble.exe" %command%
   ```
4. Launch DSX from Steam. The focus-rumble window opens alongside it.
5. In DSX, enable **Audio to Haptics** and check that the output device shows **CABLE Input**. Select it by hand if it doesn't.
6. Focus a game. The meter should move and your controller should rumble only for that game.

Don't set CABLE Input as your default Windows playback device. Games should keep playing to your speakers or headphones. focus-rumble only copies their audio.

## Limitations

- Needs Windows 11, or a Windows 10 / Server build 20348 or later, because process loopback (`PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`) isn't available earlier. Consumer Windows 10 22H2 (build 19045) is too old.
- Only audio from the focused process tree is captured. If a game plays its audio through a separate, unrelated process (some launchers or anti-cheat wrappers do), that audio is missed.
- Profiles are patched only when DSX isn't already running, so DSX reads them on start. If DSX is already open, the device has to be selected in DSX by hand.
- Profiles aren't restored on exit. Copy them back from `focus-rumble-backup` if you want the old devices back.

## Building

```
cargo build --release
```

CI (`.github/workflows/ci.yml`) builds, lints and tests on `windows-latest` and uploads the exe. Pushing a `v*` tag attaches the exe to a GitHub Release. The DSX profile patcher and the meter/FIFO math are plain Rust, so `cargo test` also runs on Linux and macOS.
