//! Foreground-window tracking via `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`.

use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, OnceLock};

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetForegroundWindow, GetMessageW,
    GetWindowThreadProcessId, MSG, TranslateMessage, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS,
};
use windows::core::PWSTR;

use crate::shared::{self, Shared};

// WinEvent callbacks carry no user pointer.
static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

pub fn spawn(shared: Arc<Shared>) {
    let _ = SHARED.set(shared);
    std::thread::Builder::new()
        .name("focus".into())
        .spawn(|| unsafe {
            on_foreground(GetForegroundWindow());
            let _hook = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(win_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            // Out-of-context hooks are delivered through this thread's message queue.
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .expect("spawn focus thread");
}

unsafe extern "system" fn win_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    on_foreground(hwnd);
}

fn on_foreground(hwnd: HWND) {
    let Some(shared) = SHARED.get() else { return };
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    // Focusing our own window keeps the previous target.
    if pid == 0 || pid == std::process::id() {
        return;
    }
    shared::set(
        &shared.target_name,
        exe_name(pid).unwrap_or_else(|| "?".into()),
    );
    shared.target_pid.store(pid, Relaxed);
}

fn exe_name(pid: u32) -> Option<String> {
    Some(
        Path::new(&full_path(pid)?)
            .file_name()?
            .to_string_lossy()
            .into_owned(),
    )
}

/// Full image paths of all running processes we can query.
pub fn running_exe_paths() -> Vec<String> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut e = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut more = Process32FirstW(snap, &mut e).is_ok();
        while more {
            if let Some(p) = full_path(e.th32ProcessID) {
                out.push(p);
            }
            more = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

fn full_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        ok.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}
