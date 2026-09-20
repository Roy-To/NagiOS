#![no_std]

//! Nagi-owned boundary types for the Servo embedder.
//!
//! This crate deliberately contains no browser implementation. It translates
//! the real Nagi Surface/Input contracts into bounded data that the eventual
//! Servo `RenderingContext`, `WebViewDelegate`, and `EventLoopWaker` adapter
//! will consume. It never creates a host window or substitutes another engine.

use core::sync::atomic::{AtomicBool, Ordering};

use nagi_abi::{
    DisplayInfo, InputEvent, INPUT_EVENT_ABS, INPUT_EVENT_KEY, INPUT_EVENT_REL, INPUT_REL_X,
    INPUT_REL_Y, PIXEL_FORMAT_RGBA8888, SURFACE_BYTES, SURFACE_HEIGHT, SURFACE_WIDTH,
};

const PIXEL_BYTES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    InvalidSurface,
    InvalidFrame,
    FrameTooLarge,
}

/// A bounded view of the Nagi-owned display Surface.
pub struct NagiSurface {
    address: *mut u8,
    bytes: usize,
    width: u32,
    height: u32,
    stride: usize,
    #[cfg_attr(not(target_os = "nagi"), allow(dead_code))]
    capability: u64,
}

impl NagiSurface {
    /// Acquire the Surface description from the guest display service.
    ///
    /// The kernel supplies the address and bounds through `display_info`; the
    /// returned capability is only accepted by the kernel again on `present`.
    /// No host framebuffer or host pointer is consulted.
    #[cfg(target_os = "nagi")]
    pub fn acquire(capability: u64) -> Option<Self> {
        let mut info = DisplayInfo::default();
        if !libnagi::display_info(&mut info) {
            return None;
        }
        Self::from_info(info, capability).ok()
    }

    #[cfg_attr(not(target_os = "nagi"), allow(dead_code))]
    fn from_info(info: DisplayInfo, capability: u64) -> Result<Self, FrameError> {
        if info.surface_address == 0
            || info.surface_bytes as usize > SURFACE_BYTES
            || info.width != SURFACE_WIDTH
            || info.height != SURFACE_HEIGHT
            || info.stride < info.width * PIXEL_BYTES as u32
            || info.pixel_format != PIXEL_FORMAT_RGBA8888
            || capability == 0
        {
            return Err(FrameError::InvalidSurface);
        }
        let stride = info.stride as usize;
        let required = stride
            .checked_mul(info.height as usize)
            .ok_or(FrameError::InvalidSurface)?;
        if required > info.surface_bytes as usize {
            return Err(FrameError::InvalidSurface);
        }
        Ok(Self {
            address: info.surface_address as *mut u8,
            bytes: info.surface_bytes as usize,
            width: info.width,
            height: info.height,
            stride,
            capability,
        })
    }

    /// Copy one Servo RGBA frame into the authorized Surface.
    ///
    /// Servo's rendering adapter must provide a complete, tightly bounded
    /// source frame. The copy is row-bounded and never reads beyond either
    /// the source frame or the Surface description.
    pub fn copy_rgba_frame(
        &mut self,
        frame: &[u8],
        frame_width: u32,
        frame_height: u32,
        frame_stride: usize,
    ) -> Result<usize, FrameError> {
        let row_bytes = (frame_width as usize)
            .checked_mul(PIXEL_BYTES)
            .ok_or(FrameError::InvalidFrame)?;
        if frame_width != self.width || frame_height != self.height || frame_stride < row_bytes {
            return Err(FrameError::InvalidFrame);
        }
        let source_bytes = frame_stride
            .checked_mul(frame_height as usize)
            .ok_or(FrameError::InvalidFrame)?;
        if source_bytes > frame.len() {
            return Err(FrameError::InvalidFrame);
        }
        let destination_bytes = self
            .stride
            .checked_mul(self.height as usize)
            .ok_or(FrameError::InvalidSurface)?;
        if destination_bytes > self.bytes {
            return Err(FrameError::InvalidSurface);
        }
        for row in 0..self.height as usize {
            let source = unsafe { frame.as_ptr().add(row * frame_stride) };
            let destination = unsafe { self.address.add(row * self.stride) };
            unsafe { core::ptr::copy_nonoverlapping(source, destination, row_bytes) };
        }
        Ok(row_bytes * self.height as usize)
    }

    /// Present the copied frame through the real capability-checked display
    /// syscall. The kernel remains the authority for presentation.
    #[cfg(target_os = "nagi")]
    pub fn present(&self) -> bool {
        libnagi::display_present(self.capability)
    }

    #[cfg(test)]
    fn for_test(buffer: &mut [u8], capability: u64) -> Self {
        Self::from_info(
            DisplayInfo {
                surface_address: buffer.as_mut_ptr() as u64,
                surface_bytes: buffer.len() as u32,
                width: SURFACE_WIDTH,
                height: SURFACE_HEIGHT,
                stride: SURFACE_WIDTH * PIXEL_BYTES as u32,
                pixel_format: PIXEL_FORMAT_RGBA8888,
            },
            capability,
        )
        .unwrap()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserInput {
    MouseMove { x: u32, y: u32 },
    MouseButton { button: u16, pressed: bool },
    Key { code: u16, pressed: bool },
}

/// Translate device events without granting the browser raw device access.
pub struct InputBridge {
    x: i32,
    y: i32,
}

impl InputBridge {
    pub const fn new() -> Self {
        Self { x: 0, y: 0 }
    }

    pub fn translate(&mut self, event: InputEvent) -> Option<BrowserInput> {
        match event.event_type {
            INPUT_EVENT_REL if event.code == INPUT_REL_X || event.code == INPUT_REL_Y => {
                if event.code == INPUT_REL_X {
                    self.x = (self.x + event.value).clamp(0, SURFACE_WIDTH as i32 - 1);
                } else {
                    self.y = (self.y + event.value).clamp(0, SURFACE_HEIGHT as i32 - 1);
                }
                Some(BrowserInput::MouseMove {
                    x: self.x as u32,
                    y: self.y as u32,
                })
            }
            INPUT_EVENT_ABS if event.code == INPUT_REL_X || event.code == INPUT_REL_Y => {
                if event.code == INPUT_REL_X {
                    self.x = event.value.clamp(0, SURFACE_WIDTH as i32 - 1);
                } else {
                    self.y = event.value.clamp(0, SURFACE_HEIGHT as i32 - 1);
                }
                Some(BrowserInput::MouseMove {
                    x: self.x as u32,
                    y: self.y as u32,
                })
            }
            INPUT_EVENT_KEY => Some(BrowserInput::Key {
                code: event.code,
                pressed: event.value != 0,
            }),
            _ => None,
        }
    }
}

impl Default for InputBridge {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe wake state for the Servo-owned event-loop thread.
///
/// The eventual Servo `EventLoopWaker` implementation will call `wake` from
/// worker threads and consume the edge on the owning guest thread before
/// calling `Servo::spin_event_loop()`.
pub struct EventLoopSignal {
    pending: AtomicBool,
}

impl EventLoopSignal {
    pub const fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
        }
    }

    pub fn wake(&self) {
        self.pending.store(true, Ordering::Release);
    }

    pub fn take(&self) -> bool {
        self.pending.swap(false, Ordering::AcqRel)
    }
}

impl Default for EventLoopSignal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{BrowserInput, EventLoopSignal, InputBridge, NagiSurface};
    use nagi_abi::{InputEvent, INPUT_EVENT_KEY, INPUT_EVENT_REL, INPUT_REL_X, SURFACE_BYTES};
    use std::vec;

    #[test]
    fn copies_a_full_rgba_frame_with_bounded_rows() {
        let mut surface = vec![0_u8; SURFACE_BYTES];
        let mut adapter = NagiSurface::for_test(&mut surface, 1);
        let frame = vec![0x5a_u8; SURFACE_BYTES];

        assert_eq!(
            adapter.copy_rgba_frame(&frame, 320, 200, 320 * 4),
            Ok(SURFACE_BYTES)
        );
        assert_eq!(surface[0], 0x5a);
        assert_eq!(surface[SURFACE_BYTES - 1], 0x5a);
    }

    #[test]
    fn rejects_a_frame_that_would_read_past_its_source() {
        let mut surface = vec![0_u8; SURFACE_BYTES];
        let mut adapter = NagiSurface::for_test(&mut surface, 1);
        assert!(adapter
            .copy_rgba_frame(&[0; 320 * 4], 320, 200, 320 * 4)
            .is_err());
    }

    #[test]
    fn translates_real_input_events_to_bounded_browser_input() {
        let mut bridge = InputBridge::new();
        assert_eq!(
            bridge.translate(InputEvent {
                event_type: INPUT_EVENT_REL,
                code: INPUT_REL_X,
                value: 10,
            }),
            Some(BrowserInput::MouseMove { x: 10, y: 0 })
        );
        assert_eq!(
            bridge.translate(InputEvent {
                event_type: INPUT_EVENT_KEY,
                code: 30,
                value: 1,
            }),
            Some(BrowserInput::Key {
                code: 30,
                pressed: true
            })
        );
    }

    #[test]
    fn wake_signal_is_consumed_once() {
        let signal = EventLoopSignal::new();
        assert!(!signal.take());
        signal.wake();
        assert!(signal.take());
        assert!(!signal.take());
    }
}
