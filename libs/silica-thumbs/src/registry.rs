use std::{
    ffi::OsStr,
    os::windows::prelude::OsStrExt,
    sync::atomic::Ordering,
};
use windows::core::*;
use windows::Win32::{
    Foundation::*,
    System::{
        LibraryLoader::GetModuleFileNameW,
        Registry::{RegDeleteKeyExW, RegOpenKeyExW, RegQueryValueExW, *},
    },
    UI::Shell,
};

use crate::guards::RegistryKeyGuard;
use crate::logging::{log_message, ENABLE_DEBUG_LOGGING};

// This is the ONLY definition you need. It works for both 32-bit and 64-bit.
pub const WRITE_FLAGS: REG_SAM_FLAGS = KEY_WRITE;

// This is our thumbnail provider's unique Class ID (CLSID).
// Use a new GUID for your own projects!
pub const CLSID_PROCREATE_THUMBNAIL_PROVIDER: GUID = GUID::from_u128(0x73ea4b0c_6957_491f_aada_464a2b7dd539);

/// Helper to convert a Rust string slice to a null-terminated UTF-16 wide string.
fn to_pcwstr(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Generic function to read registry values from HKEY_CLASSES_ROOT\.procreate
/// Returns the value as a u32 if it exists and is a valid DWORD, otherwise returns None
pub fn read_procreate_registry_dword(value_name: &str) -> Option<u32> {
    let mut procreate_key: HKEY = HKEY::default();
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            w!(".procreate"),
            Some(0),
            KEY_READ,
            &mut procreate_key,
        )
    };

    if result.is_ok() {
        let procreate_key_guard = RegistryKeyGuard(procreate_key);

        let mut value: u32 = 0;
        let mut value_size = std::mem::size_of::<u32>() as u32;
        let mut value_type = REG_DWORD;

        // Convert the value name to a wide string
        let wide_name = to_pcwstr(value_name);

        let query_result = unsafe {
            RegQueryValueExW(
                procreate_key_guard.0,
                PCWSTR(wide_name.as_ptr()),
                None,
                Some(&mut value_type),
                Some(&mut value as *mut u32 as *mut u8),
                Some(&mut value_size),
            )
        };

        // Only return the value if it exists, is a DWORD, and has the expected size
        if query_result.is_ok()
            && value_type == REG_DWORD
            && value_size == std::mem::size_of::<u32>() as u32
        {
            return Some(value);
        } else if !query_result.is_ok() {
            log_message(&format!(
                "Registry read failed for '{}': {:?}",
                value_name, query_result
            ));
        }
    } // Registry key automatically closed here by RegistryKeyGuard

    return None;
}

// Checks registry for setting for whether to enable debug logging
pub fn check_debug_logging_registry() {
    // Note: We can't log here initially since logging might not be enabled yet
    let enable_debug = match read_procreate_registry_dword("win_procreate_thumbs_enable_debug_log") {
        Some(1) => true, // Only enable debug logging if value exists and equals 1
        _ => false,      // Default to disabled for any other case (missing, 0, or other values)
    };

    ENABLE_DEBUG_LOGGING.store(enable_debug, Ordering::Relaxed);

    // Now we can log since the flag is set
    if enable_debug {
        log_message("Debug logging ENABLED via registry");
    }
}

pub fn create_registry_keys() -> Result<()> {
    log_message("create_registry_keys: Starting registry key creation");

    let clsid_string = format!("{{{CLSID_PROCREATE_THUMBNAIL_PROVIDER:?}}}");
    let dll_path = get_dll_path()?;
    log_message(&format!(
        "create_registry_keys: Using CLSID: {} and DLL path: {}",
        clsid_string, dll_path
    ));

    // Create CLSID\{our-clsid}
    // log_message("create_registry_keys: Creating CLSID root key");
    let clsid_root_key = RegistryKeyGuard::create_root_key(HKEY_CLASSES_ROOT, &w!("CLSID"))?;

    log_message("create_registry_keys: Creating CLSID subkey and setting description");
    let clsid_key = clsid_root_key.create_subkey(&PCWSTR(to_pcwstr(&clsid_string).as_ptr()))?;
    clsid_key.set_string_value("", "Procreate Thumbnail Provider (Rust)")?;

    // Create CLSID\{our-clsid}\InprocServer32
    log_message("create_registry_keys: Creating InprocServer32 key");
    let inproc_key = clsid_key.create_subkey(&w!("InprocServer32"))?;
    inproc_key.set_string_value("", &dll_path)?;
    inproc_key.set_string_value("ThreadingModel", "Apartment")?;

    // Associate with .procreate files
    log_message("create_registry_keys: Associating with .procreate files");
    let procreate_root_key = RegistryKeyGuard(HKEY_CLASSES_ROOT).create_subkey(&w!(".procreate"))?;
    let procreate_shellex_key = procreate_root_key.create_subkey(&w!("shellex"))?;
    let procreate_handler_key =
        procreate_shellex_key.create_subkey(&w!("{E357FCCD-A995-4576-B01F-234630154E96}"))?;
    procreate_handler_key.set_string_value("", &clsid_string)?;

    // log_message("create_registry_keys: Notifying shell of association changes");
    unsafe { Shell::SHChangeNotify(Shell::SHCNE_ASSOCCHANGED, Shell::SHCNF_IDLIST, None, None) };

    // log_message("create_registry_keys: Successfully completed registry key creation");
    Ok(())
}

fn get_dll_path() -> Result<String> {
    use crate::MODULE_HANDLE;
    use std::sync::atomic::Ordering;

    let handle_ptr: *mut std::ffi::c_void = MODULE_HANDLE.load(Ordering::Acquire);

    // Check for null pointer to avoid potential crashes
    if handle_ptr.is_null() {
        return Err(Error::new(E_FAIL, "MODULE_HANDLE is null; DLL not loaded?"));
    }

    let handle: HMODULE = HMODULE(handle_ptr);
    let mut path = vec![0u16; MAX_PATH as usize];
    let len: u32 = unsafe { GetModuleFileNameW(Some(handle), &mut path) };

    // If the returned length is zero, it's an error
    if len == 0 {
        return Err(Error::new(E_FAIL, "GetModuleFileNameW failed (returned 0)"));
    }

    // If the returned length is equal to the buffer size, truncation may have occurred
    if (len as usize) >= path.len() {
        return Err(Error::new(
            E_FAIL,
            "DLL path is too long (truncated); registration aborted",
        ));
    }

    // Additional safety check - ensure we don't go beyond the buffer
    let len = std::cmp::min(len as usize, path.len());
    Ok(String::from_utf16_lossy(&path[..len]))
}

pub fn delete_registry_keys() -> Result<()> {
    log_message("delete_registry_keys: Starting registry key deletion");

    let clsid_string = format!("{{{CLSID_PROCREATE_THUMBNAIL_PROVIDER:?}}}");
    log_message(&format!(
        "delete_registry_keys: Deleting keys for CLSID: {}",
        clsid_string
    ));
    // Track if we encountered any real errors (not just "not found")
    let mut first_real_error: Option<Error> = None;

    // Helper closure for robust key deletion
    let mut delete_key_with_error_tracking = |key_path: PCWSTR| {
        let result =
            unsafe { RegDeleteKeyExW(HKEY_CLASSES_ROOT, key_path, WRITE_FLAGS.0, Some(0)) };
        if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
            // Success or key already gone - both fine for uninstall
        } else {
            // Real error (access denied, etc.) - remember the first one we see
            if first_real_error.is_none() {
                first_real_error = Some(Error::new(result.into(), "Registry key deletion failed"));
            }
        }
    };

    // Try to delete all keys, tracking errors but not stopping
    let inproc_path = to_pcwstr(&format!("CLSID\\{}\\InprocServer32", clsid_string));
    delete_key_with_error_tracking(PCWSTR(inproc_path.as_ptr()));

    let clsid_path = to_pcwstr(&format!("CLSID\\{}", clsid_string));
    delete_key_with_error_tracking(PCWSTR(clsid_path.as_ptr()));

    delete_key_with_error_tracking(w!(".procreate\\shellex\\{E357FCCD-A995-4576-B01F-234630154E96}"));

    // Always notify of association changes, even if some deletions failed
    unsafe { Shell::SHChangeNotify(Shell::SHCNE_ASSOCCHANGED, Shell::SHCNF_IDLIST, None, None) };

    // Now propagate the first real error we encountered, if any
    match first_real_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
