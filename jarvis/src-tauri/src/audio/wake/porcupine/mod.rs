//! Porcupine (`libpv_porcupine`) backend — loads DLL at runtime, reads access key from OS keychain.

#[cfg(all(feature = "porcupine", target_os = "windows"))]
mod windows;

// Re-export for T4-5 orchestrator; crate root does not alias yet.
#[cfg(all(feature = "porcupine", target_os = "windows"))]
#[allow(unused_imports)]
pub use windows::PorcupineBackend;

#[cfg(all(feature = "porcupine", not(target_os = "windows")))]
mod stub;

#[cfg(all(feature = "porcupine", not(target_os = "windows")))]
#[allow(unused_imports)]
pub use stub::PorcupineBackend;
