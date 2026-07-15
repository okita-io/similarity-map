//! ONNX Runtime dynamic library discovery and initialization.
//!
//! The `ort` crate is built with `load-dynamic`. We must load `libonnxruntime` from an
//! explicit path before any session APIs run. If the dylib is missing, older `ort` versions
//! can deadlock inside error handling — so we only call `ort::init_from` after verifying the
//! file exists.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::types::{AppError, ModelError};

static ORT_INIT: OnceLock<Result<(), AppError>> = OnceLock::new();

/// Default ONNX Runtime shared library name for this platform.
pub fn dylib_filename() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "onnxruntime.dll"
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        "libonnxruntime.so"
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        "libonnxruntime.dylib"
    }
    #[cfg(not(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    {
        "libonnxruntime.so"
    }
}

/// Candidate paths to probe, in order. `ORT_DYLIB_PATH` is checked first when set.
pub fn dylib_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(env_path) = std::env::var("ORT_DYLIB_PATH") {
        if !env_path.is_empty() {
            paths.push(PathBuf::from(env_path));
        }
    }

    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from("/opt/homebrew/lib/libonnxruntime.dylib"));
        paths.push(PathBuf::from("/usr/local/lib/libonnxruntime.dylib"));
    }

    #[cfg(target_os = "linux")]
    {
        paths.push(PathBuf::from("/usr/lib/x86_64-linux-gnu/libonnxruntime.so"));
        paths.push(PathBuf::from(
            "/usr/lib/aarch64-linux-gnu/libonnxruntime.so",
        ));
        paths.push(PathBuf::from("/usr/local/lib/libonnxruntime.so"));
        paths.push(PathBuf::from("/usr/lib/libonnxruntime.so"));
    }

    #[cfg(target_os = "windows")]
    {
        paths.push(PathBuf::from("onnxruntime.dll"));
    }

    paths
}

/// Returns the first existing ONNX Runtime dylib from [`dylib_search_paths`].
pub fn resolve_dylib_path() -> Option<PathBuf> {
    dylib_search_paths().into_iter().find(|path| path.is_file())
}

/// Load and initialize ONNX Runtime once per process.
///
/// # Errors
/// Returns `AppError::Model` if the dylib cannot be found or fails to load.
pub fn ensure_loaded() -> Result<(), AppError> {
    ORT_INIT.get_or_init(try_load).clone()
}

fn try_load() -> Result<(), AppError> {
    let path = resolve_dylib_path().ok_or_else(missing_runtime_error)?;

    if !path.is_file() {
        return Err(missing_runtime_error());
    }

    ort::init_from(&path)
        .map_err(|e| load_failed_error(&path, &e.to_string()))?
        .commit();

    log::info!("ONNX Runtime loaded from {}", path.display());

    Ok(())
}

fn missing_runtime_error() -> AppError {
    let name = dylib_filename();
    let hint = install_hint();
    AppError::Model(ModelError {
        message: format!("ONNX Runtime shared library ({name}) not found. {hint}"),
        recoverable: false,
    })
}

fn load_failed_error(path: &Path, detail: &str) -> AppError {
    AppError::Model(ModelError {
        message: format!(
            "Failed to load ONNX Runtime from `{}`: {detail}. {}",
            path.display(),
            install_hint()
        ),
        recoverable: false,
    })
}

fn install_hint() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Install with `brew install onnxruntime`, or set ORT_DYLIB_PATH to the full path of libonnxruntime.dylib (e.g. export ORT_DYLIB_PATH=\"$(brew --prefix onnxruntime)/lib/libonnxruntime.dylib\")."
    }
    #[cfg(target_os = "linux")]
    {
        "Install the onnxruntime package for your distro, or set ORT_DYLIB_PATH to the full path of libonnxruntime.so."
    }
    #[cfg(target_os = "windows")]
    {
        "Install ONNX Runtime and set ORT_DYLIB_PATH to the full path of onnxruntime.dll."
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "Set ORT_DYLIB_PATH to the full path of the ONNX Runtime shared library."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dylib_filename_is_non_empty() {
        assert!(!dylib_filename().is_empty());
    }

    #[test]
    fn ort_dylib_path_env_is_first_candidate() {
        std::env::set_var("ORT_DYLIB_PATH", "/custom/libonnxruntime.dylib");
        let paths = dylib_search_paths();
        assert_eq!(
            paths.first().map(|p| p.as_os_str()),
            Some(std::ffi::OsStr::new("/custom/libonnxruntime.dylib"))
        );
        std::env::remove_var("ORT_DYLIB_PATH");
    }
}
