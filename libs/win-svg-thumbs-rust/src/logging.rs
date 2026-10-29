use std::{
    io::Write,
    path::PathBuf,
    sync::{atomic::Ordering, OnceLock},
};
use windows::Win32::{
    Globalization::{GetTimeFormatEx, TIME_FORMAT_FLAGS},
    System::SystemInformation::GetLocalTime,
    UI::Shell::{self, FOLDERID_Desktop, SHGetKnownFolderPath},
};

use crate::guards::CoTaskMemFreeGuard;

// Global flag for whether to enable debug logging
pub static ENABLE_DEBUG_LOGGING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// A global OnceLock for the log file path, initialized only once
static LOG_FILE_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn log_message(message: &str) {
    if !ENABLE_DEBUG_LOGGING.load(Ordering::Relaxed) {
        return;
    }

    // get_or_init will only execute the closure ONCE, the very first time it's called.
    // All subsequent calls will return the cached value instantly.
    let log_path_option = LOG_FILE_PATH.get_or_init(|| {
        let known_folder_flags = Shell::KNOWN_FOLDER_FLAG::default(); // Use default flags, no special options
        let desktop_path_pwstr =
            match unsafe { SHGetKnownFolderPath(&FOLDERID_Desktop, known_folder_flags, None) } {
                Ok(path) => path,
                Err(_) => return None, // Initialization failed, cache 'None'
            };

        let desktop_path_guard = CoTaskMemFreeGuard(desktop_path_pwstr);

        let mut path = match unsafe { desktop_path_guard.0.to_string() } {
            Ok(s) => PathBuf::from(s),
            Err(_) => return None, // Conversion failed, cache 'None'
        };

        path.push("win_procreate_thumb_debug_log.txt");
        Some(path) // Success! Cache the full path.
                   // --- End of one-time execution block ---
    });

    // Now, use the cached path.
    // If initialization failed, log_path_option will be &None, and we'll do nothing.
    if let Some(log_path) = log_path_option {
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
        {
            Ok(mut file) => {
                let pid = std::process::id();
                let tid = std::thread::current().id();
                let time_str = get_formatted_time_string_win_api();

                let _ = writeln!(
                    file,
                    "[PID: {} | TID: {:?}] [{}] {}",
                    pid, tid, time_str, message
                );
            }
            Err(_) => {
                // Opening the file failed.
            }
        }
    }
}

fn get_formatted_time_string_win_api() -> String {
    let system_time = unsafe { GetLocalTime() };
    let mut time_buffer = [0u16; 64]; // Buffer for formatted time string
    let chars_written = unsafe {
        // Returns the number of characters put into the buffer. If 0 it failed
        GetTimeFormatEx(
            None,                         // Null is LOCALE_NAME_USER_DEFAULT
            TIME_FORMAT_FLAGS::default(), // Default time format flags
            Some(&system_time),
            None, // Use default format
            Some(&mut time_buffer),
        )
    };

    if chars_written > 0 {
        return String::from_utf16_lossy(&time_buffer[..chars_written as usize - 1]);
    // -1 to remove null terminator
    } else {
        // Fallback if formatting fails
        return format!(
            "{:02}:{:02}:{:02}.{:03}",
            system_time.wHour, system_time.wMinute, system_time.wSecond, system_time.wMilliseconds
        );
    }
}
