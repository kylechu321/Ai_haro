use winapi::um::winuser::{GetForegroundWindow, GetWindowThreadProcessId};
use winapi::um::tlhelp32::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW,
    PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use winapi::um::handleapi::CloseHandle;
use winapi::shared::minwindef::FALSE;

const GAME_PROCESS: &str = "MechaBreak.exe";

pub fn is_game_running() -> bool {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == winapi::um::handleapi::INVALID_HANDLE_VALUE {
            return false;
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry) == FALSE {
            CloseHandle(snapshot);
            return false;
        }

        loop {
            let name = String::from_utf16_lossy(
                &entry.szExeFile[..entry.szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len())]
            );

            if name.eq_ignore_ascii_case(GAME_PROCESS) {
                CloseHandle(snapshot);
                return true;
            }

            if Process32NextW(snapshot, &mut entry) == FALSE {
                break;
            }
        }

        CloseHandle(snapshot);
        false
    }
}

// ← new: returns true only if Mecha Break is the focused foreground window
pub fn is_game_focused() -> bool {
    unsafe {
        let fg_hwnd = GetForegroundWindow();
        if fg_hwnd.is_null() {
            return false;
        }

        // get the process ID of whatever window is in focus
        let mut fg_pid: u32 = 0;
        GetWindowThreadProcessId(fg_hwnd, &mut fg_pid);

        // scan process list to find MechaBreak.exe's PID
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == winapi::um::handleapi::INVALID_HANDLE_VALUE {
            return false;
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry) == FALSE {
            CloseHandle(snapshot);
            return false;
        }

        loop {
            let name = String::from_utf16_lossy(
                &entry.szExeFile[..entry.szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len())]
            );

            if name.eq_ignore_ascii_case(GAME_PROCESS) && entry.th32ProcessID == fg_pid {
                CloseHandle(snapshot);
                return true;
            }

            if Process32NextW(snapshot, &mut entry) == FALSE {
                break;
            }
        }

        CloseHandle(snapshot);
        false
    }
}

pub fn wait_for_game() {
    if is_game_running() {
        println!("Mecha Break detected — starting overlay");
        return;
    }

    println!("Waiting for Mecha Break to launch...");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if is_game_running() {
            println!("Mecha Break detected — starting overlay");
            return;
        }
    }
}

pub fn watch_for_exit(on_exit: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            if !is_game_running() {
                println!("Mecha Break closed — shutting down overlay");
                on_exit();
                return;
            }
        }
    });
}