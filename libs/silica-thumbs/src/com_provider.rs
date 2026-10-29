use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use windows::core::*;
use windows::Win32::{Foundation::*, Graphics::Gdi, System::Com, UI::Shell};

use crate::ffi_guard;
use crate::image_processing::{
    create_fallback_thumbnail, extract_procreate_thumbnail_bytes, render_procreate_to_hbitmap,
};
use crate::logging::log_message;
use crate::{dll_add_ref, dll_release};

/// Wrapper around IStream that implements std::io::Read and std::io::Seek
/// This allows IStream to be used directly with Rust's standard I/O traits
pub struct IStreamWrapper<'a> {
    stream: &'a Com::IStream,
}

impl<'a> IStreamWrapper<'a> {
    /// Create a new wrapper around an IStream
    pub fn new(stream: &'a Com::IStream) -> Self {
        Self { stream }
    }
}

/// Implementation of std::io::Read for IStreamWrapper
impl<'a> Read for IStreamWrapper<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut bytes_read: u32 = 0;
        let hr = unsafe {
            self.stream.Read(
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                buf.len() as u32,
                Some(&mut bytes_read),
            )
        };

        if hr.is_ok() {
            Ok(bytes_read as usize)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("IStream::Read failed with HRESULT: {:?}", hr),
            ))
        }
    }
}

/// Implementation of std::io::Seek for IStreamWrapper
impl<'a> Seek for IStreamWrapper<'a> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let (offset, origin) = match pos {
            SeekFrom::Start(pos) => (pos as i64, Com::STREAM_SEEK_SET),
            SeekFrom::End(pos) => (pos, Com::STREAM_SEEK_END),
            SeekFrom::Current(pos) => (pos, Com::STREAM_SEEK_CUR),
        };

        let mut new_position: u64 = 0;
        let hr = unsafe { self.stream.Seek(offset, origin, Some(&mut new_position)) };

        if hr.is_ok() {
            Ok(new_position)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("IStream::Seek failed with HRESULT: {:?}", hr),
            ))
        }
    }
}

// =================================================================
//                 COM Thumbnail Provider Object
// =================================================================

#[implement(
    Shell::PropertiesSystem::IInitializeWithStream,
    Shell::IThumbnailProvider
)]
pub struct ThumbnailProvider {
    file_data: Mutex<Option<Arc<[u8]>>>,
}

impl Default for ThumbnailProvider {
    fn default() -> Self {
        dll_add_ref();
        log_message("ThumbnailProvider: Created new instance");
        Self {
            file_data: Mutex::new(None),
        }
    }
}

impl Drop for ThumbnailProvider {
    fn drop(&mut self) {
        log_message("ThumbnailProvider: Dropping instance");
        dll_release();
    }
}

impl Shell::PropertiesSystem::IInitializeWithStream_Impl for ThumbnailProvider_Impl {
    #[allow(non_snake_case)]
    fn Initialize(&self, pstream: Ref<'_, Com::IStream>, _grfmode: u32) -> Result<()> {
        ffi_guard!(Result<()>, {
            log_message("Initialize: Starting file data loading");

            // Guard against repeated initialization calls
            if self
                .file_data
                .lock()
                .map_err(|_| Error::new(E_FAIL, "Mutex was poisoned"))?
                .is_some()
            {
                log_message("Initialize: Error - Already initialized");
                return Err(Error::from(HRESULT::from_win32(
                    ERROR_ALREADY_INITIALIZED.0,
                )));
            }

            match pstream.as_ref() {
                Some(stream) => {
                    let buffer = extract_procreate_thumbnail_bytes(IStreamWrapper::new(stream))?;

                    log_message(&format!(
                        "Initialize: Successfully loaded {} bytes of file data",
                        buffer.len()
                    ));

                    // Convert to Arc<[u8]> to save memory overhead
                    *self
                        .file_data
                        .lock()
                        .map_err(|_| Error::new(E_FAIL, "Mutex was poisoned"))? =
                        Some(Arc::from(buffer.into_boxed_slice()));

                    log_message("Initialize: Succeeded.");
                    Ok(())
                }
                None => {
                    // This case handles if Windows passes a null stream.
                    log_message("Initialize: Error - Stream was null.");
                    Err(E_INVALIDARG.into())
                }
            }
        })
    }
}

impl Shell::IThumbnailProvider_Impl for ThumbnailProvider_Impl {
    #[allow(non_snake_case)]
    fn GetThumbnail(
        &self,
        cx: u32,
        phbmp: *mut Gdi::HBITMAP,
        pdwalpha: *mut Shell::WTS_ALPHATYPE,
    ) -> Result<()> {
        ffi_guard!(Result<()>, {
            log_message(&format!("GetThumbnail: Entered with size: {}x{}", cx, cx));

            // Initialize output parameters to safe defaults (COM contract requirement)
            // pdwalpha is set to UNKNOWN for all failure cases, only changed to ARGB on success
            unsafe {
                *phbmp = Gdi::HBITMAP(std::ptr::null_mut());
                *pdwalpha = Shell::WTSAT_UNKNOWN;
            }

            // Clone the Arc (cheap pointer copy) and release the mutex before rendering to prevent deadlocks
            let file_data = {
                let data_guard = self
                    .file_data
                    .lock()
                    .map_err(|_| Error::new(E_FAIL, "Mutex was poisoned"))?;

                match data_guard.as_ref() {
                    Some(data) => {
                        // log_message(&format!("GetThumbnail: File data is {} bytes.", data.len()));
                        Arc::clone(data) // Clone the Arc (cheap pointer copy)
                    }
                    None => {
                        log_message("GetThumbnail: Error - File data was not initialized.");
                        return Err(Error::new(E_UNEXPECTED, "File data not initialized"));
                    }
                }
            }; // Mutex lock is released here

            match render_procreate_to_hbitmap(&file_data[..], cx, cx) {
                Ok(hbitmap) => {
                    // log_message("GetThumbnail: render_file_to_hbitmap succeeded.");
                    unsafe {
                        *phbmp = hbitmap;
                        *pdwalpha = Shell::WTSAT_ARGB;
                    }
                    // log_message("GetThumbnail: Succeeded.");
                    Ok(())
                }
                Err(e) => {
                    log_message(&format!(
                        "GetThumbnail: render_file_to_hbitmap failed with error: {:?}",
                        e
                    ));

                    // Instead of returning an error, create a fallback thumbnail
                    match create_fallback_thumbnail(cx) {
                        Ok(fallback_hbitmap) => {
                            log_message(
                                "GetThumbnail: Created fallback thumbnail for invalid file.",
                            );
                            unsafe {
                                *phbmp = fallback_hbitmap;
                                *pdwalpha = Shell::WTSAT_ARGB;
                            }
                            Ok(())
                        }
                        Err(fallback_err) => {
                            log_message(&format!(
                                "GetThumbnail: Failed to create fallback thumbnail: {:?}",
                                fallback_err
                            ));
                            Err(e) // Only return error if we can't even create a fallback
                        }
                    }
                }
            }
        })
    }
}

// =================================================================
//                      COM Class Factory
// =================================================================

#[implement(Com::IClassFactory)]
pub struct ClassFactory;

impl Default for ClassFactory {
    fn default() -> Self {
        dll_add_ref();
        log_message("ClassFactory: Created new instance");
        Self {}
    }
}

impl Drop for ClassFactory {
    fn drop(&mut self) {
        log_message("ClassFactory: Dropping instance");
        dll_release();
    }
}

impl Com::IClassFactory_Impl for ClassFactory_Impl {
    #[allow(non_snake_case)]
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut std::ffi::c_void,
    ) -> Result<()> {
        ffi_guard!(Result<()>, {
            // log_message(&format!("ClassFactory::CreateInstance: Entered. Requesting interface: {:?}", unsafe { *riid }));

            // Safety checks for null pointers
            if riid.is_null() || ppvobject.is_null() {
                log_message("ClassFactory::CreateInstance: Error - Null pointer passed");
                return Err(Error::new(
                    E_POINTER,
                    "Null pointer passed to CreateInstance",
                ));
            }

            // We do not support aggregation.
            if !punkouter.is_null() {
                log_message("ClassFactory::CreateInstance: Error - Aggregation not supported.");
                return Err(Error::new(
                    CLASS_E_NOAGGREGATION,
                    "Aggregation not supported",
                ));
            }

            log_message("ClassFactory::CreateInstance: Creating ThumbnailProvider instance");

            // Create an instance of our ThumbnailProvider
            let thumbnail_provider: IUnknown = ThumbnailProvider::default().into();

            // Query for the interface requested by the caller and return it.
            let hr: HRESULT = unsafe { thumbnail_provider.query(&*riid, ppvobject) };

            if hr.is_ok() {
                Ok(())
            } else {
                log_message(&format!(
                    "ClassFactory::CreateInstance: Error - Exiting with HRESULT: {:?}",
                    hr
                ));
                Err(Error::new(hr, "Failed to query interface"))
            }
        })
    }

    #[allow(non_snake_case)]
    fn LockServer(&self, flock: BOOL) -> Result<()> {
        ffi_guard!(Result<()>, {
            if flock.as_bool() {
                log_message("ClassFactory::LockServer: Locking server (adding reference)");
                dll_add_ref();
            } else {
                log_message("ClassFactory::LockServer: Unlocking server (releasing reference)");
                dll_release();
            }
            Ok(())
        })
    }
}
