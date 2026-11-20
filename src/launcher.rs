//! Launcher mode: `focus-rumble.exe <command...>` (Steam: `"...\focus-rumble.exe" %command%`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;

use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::w;

use crate::shared::{self, Shared};
use crate::{audio, dsx, focus};

pub fn run(command: Vec<String>, shared: Arc<Shared>) {
    // Separate thread: COM there is MTA, while winit makes the UI thread STA.
    let cmd = command.clone();
    let note = thread::spawn(move || prepare_dsx(&cmd))
        .join()
        .unwrap_or_else(|_| "DSX profile update failed unexpectedly.".into());
    shared::set(&shared.launcher_note, note);

    thread::spawn(
        move || match Command::new(&command[0]).args(&command[1..]).spawn() {
            Ok(mut child) => {
                let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(0);
                std::process::exit(code);
            }
            Err(e) => {
                let note = shared::get(&shared.launcher_note);
                shared::set(
                    &shared.launcher_note,
                    format!("{note}\nFailed to start {}: {e}", command[0]),
                );
            }
        },
    );
}

fn prepare_dsx(command: &[String]) -> String {
    let Some(savefile) = dsx::locate_savefile(command, &steam_libraries()) else {
        return "DSX install not found; profiles not changed.".into();
    };
    let install = savefile.parent().unwrap_or(&savefile);
    if is_running_from(install) {
        return "DSX is already running; profiles left unchanged.".into();
    }
    let id = match audio::cable_endpoint_id() {
        Ok(Some(id)) => id,
        Ok(None) => return format!("{} not found; DSX profiles not changed.", audio::CABLE_NAME),
        Err(e) => return format!("Audio device lookup failed ({e}); DSX profiles not changed."),
    };
    let r = dsx::patch_profiles(&savefile, &id);
    let mut note = format!(
        "DSX profiles: {} switched to {}, {} already set.",
        r.patched,
        audio::CABLE_NAME,
        r.unchanged
    );
    for e in r.errors {
        note += &format!("\n{e}");
    }
    note
}

fn is_running_from(dir: &Path) -> bool {
    let dir = dir.to_string_lossy().to_lowercase();
    focus::running_exe_paths()
        .iter()
        .any(|p| p.to_lowercase().starts_with(&dir))
}

fn steam_libraries() -> Vec<PathBuf> {
    let steam = steam_path().unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Steam"));
    let vdf = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf"))
        .unwrap_or_default();
    let mut libs = dsx::parse_library_paths(&vdf);
    libs.push(steam);
    libs
}

fn steam_path() -> Option<PathBuf> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Valve\Steam"),
            w!("SteamPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    let len = (size as usize / 2).saturating_sub(1); // drop the NUL terminator
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}
