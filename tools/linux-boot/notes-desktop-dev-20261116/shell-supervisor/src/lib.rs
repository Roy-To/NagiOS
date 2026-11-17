//! Task-local consumer-side launch/bootstrap and Shell surface-owner loop.
//!
//! The Linux parent keeps the physical evdev and framebuffer descriptors. It
//! retains the physical evdev and framebuffer descriptors. The launch adapter
//! is deliberately unavailable until Task41 supplies an authorized Notes
//! bootstrap route and Task24 supplies exact process-instance terminal proof.

pub mod existing_shell_consumer;

pub mod notes_bootstrap;
pub mod shell_surface;
#[cfg(unix)]
pub mod system_language_settings;
pub mod terminal_result;

#[cfg(target_os = "linux")]
pub mod notes_child;
#[cfg(target_os = "linux")]
pub mod shell_loop;

#[cfg(test)]
mod tests {}
