//! Locating DSX profiles and pointing their audio device ids at CABLE Input.
//! Plain std code so it is unit-testable on any OS.

use std::fs;
use std::path::{Path, PathBuf};

pub const DEVICE_KEYS: [&str; 2] = [
    "audio_haptics_output_device_id",
    "audio_to_rumble_output_device_id",
];
const PROFILE_DIRS: [&str; 2] = ["Controller Profiles", "Game Profiles"];
const BACKUP_DIR: &str = "focus-rumble-backup";

/// Returns the byte range of the JSON string token starting at `start` (a `"`),
/// including both quotes.
fn string_end(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Replace the string value of every `"key": "<string>"` member, leaving all other
/// bytes (whitespace, ordering, other fields) untouched. Returns `None` if the key
/// is absent or its value is not a string.
pub fn set_string_field(json: &str, key: &str, value: &str) -> Option<String> {
    let b = json.as_bytes();
    let quoted_key = format!("\"{key}\"");
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    let mut out = String::with_capacity(json.len() + value.len());
    let (mut copied, mut i, mut found) = (0, 0, false);
    while i < b.len() {
        if b[i] != b'"' {
            i += 1;
            continue;
        }
        let end = string_end(b, i)?;
        let colon = skip_ws(b, end);
        if json[i..end] == quoted_key && b.get(colon) == Some(&b':') {
            let vstart = skip_ws(b, colon + 1);
            if b.get(vstart) == Some(&b'"') {
                let vend = string_end(b, vstart)?;
                out.push_str(&json[copied..vstart]);
                out.push('"');
                out.push_str(&escaped);
                out.push('"');
                copied = vend;
                found = true;
                i = vend;
                continue;
            }
        }
        i = end;
    }
    out.push_str(&json[copied..]);
    found.then_some(out)
}

/// Library roots from Steam's `libraryfolders.vdf` (`"path"  "D:\\SteamLibrary"` lines).
pub fn parse_library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let mut parts = line.split('"').skip(1).step_by(2);
            if parts.next()? != "path" {
                return None;
            }
            Some(PathBuf::from(parts.next()?.replace("\\\\", "\\")))
        })
        .collect()
}

/// `dir/DSX_Savefile` or `dir/*/DSX_Savefile` (the install keeps it under e.g. `Main_v3_Beta`).
pub fn find_savefile(dir: &Path) -> Option<PathBuf> {
    let direct = dir.join("DSX_Savefile");
    if direct.is_dir() {
        return Some(direct);
    }
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path().join("DSX_Savefile"))
        .find(|p| p.is_dir())
}

/// Search the launch command's paths (and the Steam library each lives in), then the
/// given Steam library roots, for the DSX save folder.
pub fn locate_savefile(command: &[String], steam_libraries: &[PathBuf]) -> Option<PathBuf> {
    let from_command = command
        .iter()
        .map(Path::new)
        .filter(|p| p.is_absolute())
        .flat_map(Path::ancestors)
        .flat_map(|a| {
            [
                a.to_path_buf(),
                a.join("steamapps").join("common").join("DSX"),
            ]
        });
    let from_steam = steam_libraries
        .iter()
        .map(|lib| lib.join("steamapps").join("common").join("DSX"));
    from_command
        .chain(from_steam)
        .find_map(|d| find_savefile(&d))
}

#[derive(Debug, Default, PartialEq)]
pub struct PatchReport {
    pub patched: usize,
    pub unchanged: usize,
    pub errors: Vec<String>,
}

/// Back up (first time only) and patch every `.dsx` profile under the save folder.
pub fn patch_profiles(savefile: &Path, endpoint_id: &str) -> PatchReport {
    let mut report = PatchReport::default();
    let config = savefile.join("Configuration Files");
    for sub in PROFILE_DIRS {
        let Ok(entries) = fs::read_dir(config.join(sub)) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path
                .extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case("dsx"))
            {
                continue;
            }
            match patch_file(&path, &config.join(BACKUP_DIR).join(sub), endpoint_id) {
                Ok(true) => report.patched += 1,
                Ok(false) => report.unchanged += 1,
                Err(e) => report.errors.push(format!("{}: {e}", path.display())),
            }
        }
    }
    report
}

fn patch_file(path: &Path, backup_dir: &Path, endpoint_id: &str) -> std::io::Result<bool> {
    let original = fs::read_to_string(path)?;
    let patched = DEVICE_KEYS.iter().fold(original.clone(), |text, key| {
        set_string_field(&text, key, endpoint_id).unwrap_or(text)
    });
    if patched == original {
        return Ok(false);
    }
    // Keep the very first backup so the user's original settings are never overwritten.
    let backup = backup_dir.join(path.file_name().unwrap_or_default());
    if !backup.exists() {
        fs::create_dir_all(backup_dir)?;
        fs::copy(path, &backup)?;
    }
    fs::write(path, patched)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/profile.dsx");
    const CABLE: &str = "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555555}";

    #[test]
    fn patches_only_device_ids() {
        let a = set_string_field(FIXTURE, DEVICE_KEYS[0], CABLE).unwrap();
        let b = set_string_field(&a, DEVICE_KEYS[1], CABLE).unwrap();
        let expected = FIXTURE
            .replace(
                "\"audio_haptics_output_device_id\": \"\"",
                &format!("\"audio_haptics_output_device_id\": \"{CABLE}\""),
            )
            .replace(
                "{0.0.0.00000000}.{9a1c3e2b-7d4f-4c1a-8e6b-2f5d9c0b7a31}",
                CABLE,
            );
        assert_eq!(b, expected);
        // A string value that merely equals the key name is not a key.
        assert!(b.contains("\"note\": \"audio_haptics_output_device_id\""));
    }

    #[test]
    fn missing_or_non_string_key_is_none() {
        assert_eq!(set_string_field(FIXTURE, "nope", CABLE), None);
        assert_eq!(set_string_field(r#"{"k": 5}"#, "k", "x"), None);
        assert_eq!(
            set_string_field(r#"{"k" :"a\"b", "j":"k"}"#, "k", "x\"y").unwrap(),
            r#"{"k" :"x\"y", "j":"k"}"#
        );
    }

    #[test]
    fn parses_libraryfolders_vdf() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}\n";
        assert_eq!(
            parse_library_paths(vdf),
            vec![
                PathBuf::from("C:\\Program Files (x86)\\Steam"),
                PathBuf::from("D:\\SteamLibrary")
            ]
        );
    }

    #[test]
    fn patches_profile_tree_with_backup() {
        let root = std::env::temp_dir().join(format!("focus-rumble-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let install = root.join("steamapps/common/DSX/Main_v3_Beta");
        let config = install.join("DSX_Savefile/Configuration Files");
        for sub in PROFILE_DIRS {
            fs::create_dir_all(config.join(sub)).unwrap();
            fs::write(config.join(sub).join("p.dsx"), FIXTURE).unwrap();
        }
        fs::write(config.join("Game Profiles/readme.txt"), "x").unwrap();

        let exe = install.join("DSX.exe").to_string_lossy().into_owned();
        let savefile = locate_savefile(&[exe], &[]).unwrap();
        assert_eq!(savefile, install.join("DSX_Savefile"));
        assert_eq!(
            locate_savefile(&[], std::slice::from_ref(&root)).unwrap(),
            savefile
        );

        let r = patch_profiles(&savefile, CABLE);
        assert_eq!((r.patched, r.unchanged, r.errors.len()), (2, 0, 0));
        let backup = config.join(BACKUP_DIR).join("Controller Profiles/p.dsx");
        assert_eq!(fs::read_to_string(&backup).unwrap(), FIXTURE);
        let patched = fs::read_to_string(config.join("Game Profiles/p.dsx")).unwrap();
        assert_eq!(patched.matches(CABLE).count(), 2);

        // Second run: nothing to change, original backup kept.
        let r = patch_profiles(&savefile, CABLE);
        assert_eq!((r.patched, r.unchanged), (0, 2));
        assert_eq!(fs::read_to_string(&backup).unwrap(), FIXTURE);
        fs::remove_dir_all(&root).unwrap();
    }
}
