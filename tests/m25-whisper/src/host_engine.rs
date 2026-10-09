//! Host model loader for the real-engine harness. Reads the locked model file
//! from the host filesystem; used only for host measurements.

use core::ffi::c_void;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use crate::provider_ffi::AdapterEngine;
use crate::provider_session::WhisperEngineLoader;

struct HostModelReader {
    file: File,
    offset: u64,
    length: u64,
    fail_at: Option<u64>,
    failed: bool,
}

unsafe extern "C" fn host_read(
    context: *mut c_void,
    destination: *mut c_void,
    size: usize,
) -> usize {
    if context.is_null() || destination.is_null() || size == 0 {
        return 0;
    }
    let reader = unsafe { &mut *context.cast::<HostModelReader>() };
    if reader.failed {
        return 0;
    }
    if reader
        .fail_at
        .is_some_and(|limit| reader.offset.saturating_add(size as u64) > limit)
    {
        reader.failed = true;
        return 0;
    }
    let output = unsafe { core::slice::from_raw_parts_mut(destination.cast::<u8>(), size) };
    let mut filled = 0;
    while filled < size {
        match reader.file.read(&mut output[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(_) => {
                reader.failed = true;
                return 0;
            }
        }
    }
    reader.offset += filled as u64;
    filled
}

unsafe extern "C" fn host_eof(context: *mut c_void) -> bool {
    if context.is_null() {
        return true;
    }
    let reader = unsafe { &*context.cast::<HostModelReader>() };
    reader.offset >= reader.length
}

unsafe extern "C" fn host_close(_context: *mut c_void) {}

/// Loads a fresh whisper.cpp context from `model` on each `load`. Load
/// attempt `inject_read_failure_on_attempt` (1-based) fails its model read
/// after `inject_read_failure_after_bytes`, exercising the adapter's real
/// load-failure path.
pub struct HostModelLoader {
    pub model: PathBuf,
    pub expected_bytes: u64,
    pub attempts: u32,
    pub inject_read_failure_on_attempt: Option<u32>,
    pub inject_read_failure_after_bytes: u64,
}

impl HostModelLoader {
    pub fn new(model: PathBuf, expected_bytes: u64) -> Self {
        Self {
            model,
            expected_bytes,
            attempts: 0,
            inject_read_failure_on_attempt: None,
            inject_read_failure_after_bytes: 64 << 20,
        }
    }
}

impl WhisperEngineLoader for HostModelLoader {
    type Engine = AdapterEngine;

    fn load(&mut self) -> Option<AdapterEngine> {
        self.attempts += 1;
        let file = File::open(&self.model).ok()?;
        let length = file.metadata().ok()?.len();
        if length != self.expected_bytes {
            return None;
        }
        let fail_at = (self.inject_read_failure_on_attempt == Some(self.attempts))
            .then_some(self.inject_read_failure_after_bytes);
        let mut reader = HostModelReader {
            file,
            offset: 0,
            length,
            fail_at,
            failed: false,
        };
        let engine = unsafe {
            AdapterEngine::init(
                (&mut reader as *mut HostModelReader).cast(),
                host_read,
                host_eof,
                host_close,
            )
        };
        if reader.failed {
            return None;
        }
        engine
    }
}
