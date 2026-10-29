use std::ffi::OsStr;
use std::os::windows::prelude::OsStrExt;
use windows::core::*;
use windows::Win32::{
    Foundation::*,
    Graphics::Gdi,
    System::{
        Com,
        Registry::{RegCloseKey, RegCreateKeyExW, RegSetValueExW, *},
    },
};

// RAII wrapper for HBITMAP - automatically calls DeleteObject when dropped
pub struct HBitmapGuard(pub Gdi::HBITMAP);

impl HBitmapGuard {
    // Create a new guard. Takes ownership of the handle.
    pub fn new(handle: Gdi::HBITMAP) -> Self {
        Self(handle)
    }

    // Release ownership of the handle (e.g., when transferring to the Shell).
    pub fn release(mut self) -> Gdi::HBITMAP {
        let handle = self.0;
        // Set the internal handle to null so Drop doesn't delete it.
        self.0 = Gdi::HBITMAP(std::ptr::null_mut());
        handle
    }
}

impl Drop for HBitmapGuard {
    fn drop(&mut self) {
        // Only delete if the handle is not null/invalid (i.e., it hasn't been released).
        if !self.0.is_invalid() && !self.0 .0.is_null() {
            let success = unsafe { Gdi::DeleteObject(Gdi::HGDIOBJ(self.0 .0)) };
            // In a Drop implementation, we can't return an error, so just ignore failure
            // In production, you might want to log this for debugging
            let _ = success;
        }
    }
}

// RAII Guard to automatically free memory from SHGetKnownFolderPath
pub struct CoTaskMemFreeGuard(pub PWSTR);

impl Drop for CoTaskMemFreeGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // This is safe because the pointer was allocated by the COM task allocator
            unsafe { Com::CoTaskMemFree(Some(self.0 .0 as *const std::ffi::c_void)) };
        }
    }
}

// RAII wrapper for registry keys - automatically closes when dropped
pub struct RegistryKeyGuard(pub HKEY);

impl Drop for RegistryKeyGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
}

impl RegistryKeyGuard {
    pub fn create_subkey(&self, name: &PCWSTR) -> Result<RegistryKeyGuard> {
        let mut key = HKEY::default();
        let mut disposition = REG_CREATE_KEY_DISPOSITION(0);
        unsafe {
            RegCreateKeyExW(
                self.0,
                *name,
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                Some(&mut disposition as *mut _),
            )
            .ok()?;
        }
        if key.is_invalid() {
            return Err(Error::new(E_FAIL, "RegCreateKeyExW returned null handle"));
        }

        Ok(RegistryKeyGuard(key))
    }

    pub fn create_root_key(hive: HKEY, name: &PCWSTR) -> Result<RegistryKeyGuard> {
        let mut key = HKEY::default();
        unsafe {
            RegCreateKeyExW(
                hive,
                *name,
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                None,
            )
            .ok()?;
        }
        Ok(RegistryKeyGuard(key))
    }

    /// Sets a REG_SZ (string) value for this registry key.
    /// The `name` can be an empty string to set the (Default) value.
    pub fn set_string_value(&self, name: &str, value: &str) -> Result<()> {
        // Convert name and value to null-terminated wide strings (Vec<u16>)
        let wide_name = to_pcwstr(name);
        let wide_value = to_pcwstr(value);

        // The size for RegSetValueExW must be in bytes, including the null terminator.
        // to_pcwstr already adds the null terminator, so wide_value.len() is correct.
        let value_size_bytes = (wide_value.len() * std::mem::size_of::<u16>()) as u32;

        unsafe {
            RegSetValueExW(
                self.0,
                PCWSTR(wide_name.as_ptr()),
                None,
                REG_SZ,
                Some(std::slice::from_raw_parts(
                    wide_value.as_ptr() as *const u8,
                    value_size_bytes as usize,
                )),
            )
            .ok()?;
        }
        Ok(())
    }
}

/// Helper to convert a Rust string slice to a null-terminated UTF-16 wide string.
fn to_pcwstr(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
