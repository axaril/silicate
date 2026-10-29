mod com_provider;
mod guards;
mod image_processing;
mod logging;
mod registry;

use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::{self, Com, Registry::HKEY_CLASSES_ROOT},
        UI::Shell,
    },
};

use com_provider::ClassFactory;
use logging::log_message;
use registry::{
    check_debug_logging_registry, create_registry_keys, delete_registry_keys, user_classes_root,
    CLSID_PROCREATE_THUMBNAIL_PROVIDER,
};

// Re-export public functions for external use
pub use image_processing::{extract_procreate_thumbnail_bytes, render_procreate_to_hbitmap};

// A global reference counter for the DLL itself.
static DLL_REFERENCES: AtomicU32 = AtomicU32::new(0);
// A global handle to the DLL module instance - using Option for safer null checking
static MODULE_HANDLE: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn dll_add_ref() {
    let new_count = DLL_REFERENCES.fetch_add(1, Ordering::Relaxed) + 1;
    log_message(&format!("DLL reference added. New count: {}", new_count));
}

pub fn dll_release() {
    let old_count = DLL_REFERENCES.fetch_sub(1, Ordering::Release);
    log_message(&format!(
        "DLL reference released. New count: {}",
        old_count - 1
    ));
}

/// Macro to wrap FFI functions with panic protection.
/// This eliminates the boilerplate code for catch_unwind and error handling.
#[macro_export]
macro_rules! ffi_guard {
    // For functions that return Result<T>
    (Result<$ret_type:ty>, $body:expr) => {{
        use std::panic::{catch_unwind, AssertUnwindSafe};
        use windows::Win32::Foundation::{E_FAIL};
        let result = catch_unwind(AssertUnwindSafe(|| $body));
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                //log_message("A PANIC occurred in FFI function.");
                Err(E_FAIL.into())
            }
        }
    }};

    // For functions that return HRESULT directly
    (HRESULT, $body:expr) => {{
        use std::panic::{catch_unwind, AssertUnwindSafe};
        use windows::Win32::Foundation::{E_FAIL};
        let result = catch_unwind(AssertUnwindSafe(|| $body));
        match result {
            Ok(hr) => hr,
            Err(_) => {
                //log_message("A PANIC occurred in FFI function.");
                E_FAIL
            }
        }
    }};

    // For functions that return BOOL
    (BOOL, $body:expr) => {{
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let result = catch_unwind(AssertUnwindSafe(|| $body));
        match result {
            Ok(success) => success.into(),
            Err(_) => {
                //log_message("A PANIC occurred in FFI function.");
                false.into()
            }
        }
    }};
}


#[no_mangle]
#[allow(non_snake_case)]
extern "system" fn DllMain(
    hinst_dll: HMODULE,
    fdw_reason: u32,
    _lpv_reserved: *const std::ffi::c_void,
) -> BOOL {
    ffi_guard!(BOOL, {
        if fdw_reason == System::SystemServices::DLL_PROCESS_ATTACH {
            MODULE_HANDLE.store(hinst_dll.0 as *mut _, Ordering::Release);
            // Check registry for debug logging preference once at startup
            check_debug_logging_registry();

            log_message("DllMain: DLL_PROCESS_ATTACH completed. DLL is loaded and initialized.");
        } else if fdw_reason == System::SystemServices::DLL_PROCESS_DETACH {
            log_message("DllMain: DLL_PROCESS_DETACH received. DLL is unloading.");
        }
        true
    })
}

#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut std::ffi::c_void,
) -> HRESULT {
    ffi_guard!(HRESULT, {
        // Check registry settings at entry point in case they changed since DLL load
        check_debug_logging_registry();

        log_message("DllGetClassObject: Entered");

        // Safety checks for null pointers
        if rclsid.is_null() || riid.is_null() || ppv.is_null() {
            log_message("DllGetClassObject: Error - Null pointer passed");
            return E_POINTER;
        }

        // Check if the caller is asking for our specific class.
        if unsafe { *rclsid } != CLSID_PROCREATE_THUMBNAIL_PROVIDER {
            log_message(&format!(
                "DllGetClassObject: Error - CLSID mismatch. Requested: {:?}, Expected: {:?}",
                unsafe { *rclsid },
                CLSID_PROCREATE_THUMBNAIL_PROVIDER
            ));
            return CLASS_E_CLASSNOTAVAILABLE;
        }

        log_message("DllGetClassObject: Creating class factory for Procreate Thumbnail Provider");

        // Create our class factory.
        let factory: Com::IClassFactory = ClassFactory::default().into();

        // Query for the interface the caller wants (usually IClassFactory) and return it.
        let hr: HRESULT = unsafe { factory.query(riid, ppv) };

        // The factory variable will automatically drop here, releasing our local reference.
        // The caller retains their reference from the query() call.

        // log_message(&format!("DllGetClassObject: Exiting with HRESULT: {:?}", hr));
        // Log only if it's an error
        if hr.is_err() {
            log_message(&format!(
                "DllGetClassObject: Error - Exiting with HRESULT: {:?}",
                hr
            ));
        } else {
            // log_message("DllGetClassObject: Succeeded.");
        }

        hr
    })
}

#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    ffi_guard!(HRESULT, {
        let ref_count = DLL_REFERENCES.load(Ordering::Acquire);

        if ref_count == 0 {
            log_message("DllCanUnloadNow: Returning S_OK - DLL can be unloaded");
            S_OK
        } else {
            log_message(&format!(
                "DllCanUnloadNow: Returning S_FALSE - DLL still has {} active references",
                ref_count
            ));
            S_FALSE
        }
    })
}

// =================================================================
//                      DLL Registration
// =================================================================

#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    ffi_guard!(HRESULT, {
        // log_message("DllRegisterServer: Starting registration");
        match create_registry_keys(HKEY_CLASSES_ROOT) {
            Ok(_) => {
                log_message("DllRegisterServer: Registration succeeded");
                S_OK
            }
            Err(e) => {
                log_message(&format!("DllRegisterServer: Registration failed: {:?}", e));
                E_FAIL
            }
        }
    })
}

#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    ffi_guard!(HRESULT, {
        // log_message("DllUnregisterServer: Starting unregistration");
        match delete_registry_keys(HKEY_CLASSES_ROOT) {
            Ok(_) => {
                log_message("DllUnregisterServer: Unregistration succeeded");
                S_OK
            }
            Err(e) => {
                log_message(&format!(
                    "DllUnregisterServer: Unregistration failed: {:?}",
                    e
                ));
                E_FAIL
            }
        }
    })
}

/// Per-user (un)registration, invoked by `regsvr32 /n /i:user [/u] <dll>`.
/// Writes to `HKEY_CURRENT_USER\Software\Classes`, so no administrator rights are needed.
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllInstall(install: BOOL, cmd_line: PCWSTR) -> HRESULT {
    ffi_guard!(HRESULT, {
        let scope = if cmd_line.is_null() {
            String::new()
        } else {
            unsafe { cmd_line.to_string() }.unwrap_or_default()
        };
        if !scope.eq_ignore_ascii_case("user") {
            log_message(&format!("DllInstall: Unsupported scope '{}'", scope));
            return E_INVALIDARG;
        }

        let result = user_classes_root().and_then(|classes_root| {
            if install.as_bool() {
                create_registry_keys(classes_root.0)
            } else {
                delete_registry_keys(classes_root.0)
            }
        });
        match result {
            Ok(_) => {
                log_message("DllInstall: Per-user (un)registration succeeded");
                S_OK
            }
            Err(e) => {
                log_message(&format!("DllInstall: Per-user (un)registration failed: {:?}", e));
                E_FAIL
            }
        }
    })
}

#[no_mangle]
// Simple function that only notifies the shell of file association changes.
pub extern "system" fn notify_shell_change() -> HRESULT {
    ffi_guard!(HRESULT, {
        // Notify the shell that file associations have changed
        unsafe {
            Shell::SHChangeNotify(Shell::SHCNE_ASSOCCHANGED, Shell::SHCNF_IDLIST, None, None)
        };
        S_OK
    })
}
