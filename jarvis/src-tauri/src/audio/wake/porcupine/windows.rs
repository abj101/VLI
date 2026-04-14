//! Windows: load `libpv_porcupine.dll` dynamically and drive Picovoice Porcupine v3 C API.

use crate::audio::wake::{
    expect_pcm_frame_len, WakeDetector, WakeError, KEYRING_PORCUPINE_ACCESS_KEY,
    KEYRING_SERVICE_PORCUPINE,
};
use keyring::Entry;
use libloading::Library;
use log::warn;
use std::ffi::CString;
use std::os::raw::c_char;
use std::path::Path;
use std::ptr;

/// Default keyword file from `download-wake-models.ps1` (built-in Picovoice keyword). Replace with
/// `jarvis_windows.ppn` from Picovoice Console for the "jarvis" wake word.
const KEYWORD_FILENAME: &str = "porcupine_windows.ppn";
const MODEL_FILENAME: &str = "porcupine_params.pv";
const DLL_NAME: &str = "libpv_porcupine.dll";

type PvPorcupineInit = unsafe extern "C" fn(
    access_key: *const c_char,
    model_path: *const c_char,
    device: *const c_char,
    num_keywords: i32,
    keyword_paths: *const *const c_char,
    sensitivities: *const f32,
    object: *mut *mut std::ffi::c_void,
) -> u32;

type PvPorcupineDelete = unsafe extern "C" fn(*mut std::ffi::c_void);
type PvPorcupineProcess = unsafe extern "C" fn(
    object: *mut std::ffi::c_void,
    pcm: *const i16,
    keyword_index: *mut i32,
) -> u32;

pub struct PorcupineBackend {
    _lib: Library,
    pv_delete: PvPorcupineDelete,
    pv_process: PvPorcupineProcess,
    ptr: *mut std::ffi::c_void,
    frame_len: usize,
}

unsafe impl Send for PorcupineBackend {}

impl PorcupineBackend {
    /// Load Porcupine from `resource_dir/porcupine/`, read access key from the OS keychain
    /// (`KEYRING_SERVICE_PORCUPINE` / `KEYRING_PORCUPINE_ACCESS_KEY`).
    pub fn try_new(resource_dir: &Path) -> Result<Self, WakeError> {
        let dir = resource_dir.join("porcupine");
        let dll_path = dir.join(DLL_NAME);
        if !dll_path.is_file() {
            warn!(
                "Porcupine: missing DLL at {}; staying in hotkey-only mode",
                dll_path.display()
            );
            return Err(WakeError::Init(format!(
                "missing Porcupine library: {}",
                dll_path.display()
            )));
        }
        let model_path = dir.join(MODEL_FILENAME);
        let keyword_path = dir.join(KEYWORD_FILENAME);
        if !model_path.is_file() || !keyword_path.is_file() {
            warn!(
                "Porcupine: missing model or keyword file under {}",
                dir.display()
            );
            return Err(WakeError::Init(format!(
                "missing Porcupine model files under {}",
                dir.display()
            )));
        }

        let entry = Entry::new(KEYRING_SERVICE_PORCUPINE, KEYRING_PORCUPINE_ACCESS_KEY)
            .map_err(|e| WakeError::Init(format!("keyring entry: {e}")))?;
        let access_key = match entry.get_password() {
            Ok(k) if !k.trim().is_empty() => k,
            Ok(_) => {
                warn!("Porcupine: empty access key in keychain; hotkey-only mode");
                return Err(WakeError::Init("Porcupine access key is not set".into()));
            }
            Err(e) => {
                warn!("Porcupine: could not read access key from keychain: {e}");
                return Err(WakeError::Init(format!("keychain: {e}")));
            }
        };

        // SAFETY: load vendor DLL from resolved path; symbols match Picovoice Porcupine 3.x.
        let lib = unsafe {
            Library::new(&dll_path)
                .map_err(|e| WakeError::Library(format!("load {}: {e}", dll_path.display())))?
        };

        let pv_init: PvPorcupineInit = unsafe {
            *lib.get(b"pv_porcupine_init\0")
                .map_err(|e| WakeError::Library(format!("symbol pv_porcupine_init: {e}")))?
        };
        let pv_delete: PvPorcupineDelete = unsafe {
            *lib.get(b"pv_porcupine_delete\0")
                .map_err(|e| WakeError::Library(format!("symbol pv_porcupine_delete: {e}")))?
        };
        let pv_process: PvPorcupineProcess = unsafe {
            *lib.get(b"pv_porcupine_process\0")
                .map_err(|e| WakeError::Library(format!("symbol pv_porcupine_process: {e}")))?
        };
        let pv_frame_len: unsafe extern "C" fn() -> i32 = unsafe {
            *lib.get(b"pv_porcupine_frame_length\0")
                .map_err(|e| WakeError::Library(format!("symbol pv_porcupine_frame_length: {e}")))?
        };

        let access_key_c = CString::new(access_key)
            .map_err(|_| WakeError::Init("Porcupine access key contains an embedded NUL".into()))?;
        let model_c = CString::new(model_path.to_string_lossy().as_bytes())
            .map_err(|_| WakeError::Init("Porcupine model path contains an embedded NUL".into()))?;
        let keyword_c = CString::new(keyword_path.to_string_lossy().as_bytes()).map_err(|_| {
            WakeError::Init("Porcupine keyword path contains an embedded NUL".into())
        })?;
        let device_c = CString::new("cpu").unwrap();
        let sensitivity: f32 = 0.5;
        let kw_ptr = keyword_c.as_ptr();
        let mut object: *mut std::ffi::c_void = ptr::null_mut();

        let status = unsafe {
            pv_init(
                access_key_c.as_ptr(),
                model_c.as_ptr(),
                device_c.as_ptr(),
                1,
                &kw_ptr,
                &sensitivity,
                &mut object,
            )
        };
        if status != 0 || object.is_null() {
            return Err(WakeError::Init(format!(
                "pv_porcupine_init failed with status {status}"
            )));
        }

        let frame_len = unsafe { pv_frame_len() as usize };
        if frame_len == 0 {
            unsafe {
                pv_delete(object);
            }
            return Err(WakeError::Init(
                "pv_porcupine_frame_length returned 0".into(),
            ));
        }

        Ok(Self {
            _lib: lib,
            pv_delete,
            pv_process,
            ptr: object,
            frame_len,
        })
    }
}

impl Drop for PorcupineBackend {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                (self.pv_delete)(self.ptr);
            }
            self.ptr = ptr::null_mut();
        }
    }
}

impl WakeDetector for PorcupineBackend {
    fn process_frame(&mut self, pcm: &[i16]) -> Result<bool, WakeError> {
        expect_pcm_frame_len(pcm.len(), self.frame_len)?;
        let mut idx: i32 = -1;
        let status = unsafe { (self.pv_process)(self.ptr, pcm.as_ptr(), &mut idx) };
        if status != 0 {
            return Err(WakeError::Process(format!(
                "pv_porcupine_process status {status}"
            )));
        }
        Ok(idx >= 0)
    }

    fn backend_name(&self) -> &'static str {
        "porcupine"
    }
}
