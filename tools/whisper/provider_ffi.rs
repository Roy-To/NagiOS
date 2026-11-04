// Nagi-owned Rust binding for the M25 whisper.cpp provider adapter
// (`tools/whisper/nagi-provider-adapter.cpp`).
//
// Shared by the guest provider (`user/nagi-init/src/m25_whisper.rs`) and the
// host harness (`tests/m25-whisper`), both of which include it next to
// `provider_session.rs`. Model bytes reach whisper.cpp only through the
// caller-supplied reader callbacks; on Nagi those read the locked artifact
// through the read-only Model Store capability.

use core::ffi::{c_char, c_void};
use core::ptr::NonNull;

use super::provider_session::{WhisperEngine, WhisperEngineError, WhisperLanguage};

/// The pinned Nagi patch keeps whisper.cpp's CPU path synchronous until the
/// target exposes a worker-pool capability (ADR 0042 budget assumes one
/// inference thread).
pub const WHISPER_THREADS: i32 = 1;

pub type WhisperReadCallback = unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> usize;
pub type WhisperEofCallback = unsafe extern "C" fn(*mut c_void) -> bool;
pub type WhisperCloseCallback = unsafe extern "C" fn(*mut c_void);

unsafe extern "C" {
    fn nagi_m25_whisper_init(
        reader_context: *mut c_void,
        read_callback: WhisperReadCallback,
        eof_callback: WhisperEofCallback,
        close_callback: WhisperCloseCallback,
    ) -> *mut c_void;
    fn nagi_m25_whisper_transcribe(
        context: *mut c_void,
        samples: *const f32,
        sample_count: i32,
        thread_count: i32,
        language: *const c_char,
        destination: *mut c_char,
        destination_size: usize,
        written: *mut usize,
    ) -> i32;
    fn nagi_m25_whisper_free(context: *mut c_void);
}

/// One loaded whisper.cpp context. Dropping it frees the context.
pub struct AdapterEngine {
    context: NonNull<c_void>,
}

impl AdapterEngine {
    /// Loads a context by streaming the model through `read_callback`.
    ///
    /// # Safety
    ///
    /// `reader_context` must be valid for the callbacks for the whole call and
    /// the callbacks must follow the `whisper_model_loader` contract. The
    /// adapter does not retain `reader_context` after returning.
    pub unsafe fn init(
        reader_context: *mut c_void,
        read_callback: WhisperReadCallback,
        eof_callback: WhisperEofCallback,
        close_callback: WhisperCloseCallback,
    ) -> Option<Self> {
        let context = unsafe {
            nagi_m25_whisper_init(reader_context, read_callback, eof_callback, close_callback)
        };
        NonNull::new(context).map(|context| Self { context })
    }
}

impl WhisperEngine for AdapterEngine {
    fn transcribe(
        &mut self,
        samples: &[f32],
        language: WhisperLanguage,
        output: &mut [u8],
    ) -> Result<usize, WhisperEngineError> {
        let sample_count =
            i32::try_from(samples.len()).map_err(|_| WhisperEngineError::InvalidInput)?;
        let mut written = 0;
        let status = unsafe {
            nagi_m25_whisper_transcribe(
                self.context.as_ptr(),
                samples.as_ptr(),
                sample_count,
                WHISPER_THREADS,
                language.code().as_ptr().cast(),
                output.as_mut_ptr().cast(),
                output.len(),
                &mut written,
            )
        };
        match WhisperEngineError::from_status(status) {
            None => Ok(written),
            Some(error) => Err(error),
        }
    }
}

impl Drop for AdapterEngine {
    fn drop(&mut self) {
        unsafe { nagi_m25_whisper_free(self.context.as_ptr()) };
    }
}
