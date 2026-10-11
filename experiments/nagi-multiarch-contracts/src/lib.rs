//! Experimental design validators; no production wiring or authority grants.
#![no_std]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Architecture {
    X86_64,
    Aarch64,
}
impl Architecture {
    pub const fn elf_machine(self) -> u16 {
        match self {
            Self::X86_64 => 62,
            Self::Aarch64 => 183,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractError {
    Topology,
    Version,
    Firmware,
    Display,
    Address,
    WritableExecutable,
    Machine,
}

/// Dense logical IDs are assigned only after validating unique hardware IDs.
/// x86 reference acceptance still requires exactly four CPUs elsewhere.
pub fn validate_topology(
    ids: &[u64],
    bootstrap: u64,
    capacity: usize,
) -> Result<(), ContractError> {
    if ids.is_empty() || ids.len() > capacity || !ids.contains(&bootstrap) {
        return Err(ContractError::Topology);
    }
    for (index, id) in ids.iter().enumerate() {
        if ids[..index].contains(id) {
            return Err(ContractError::Topology);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Firmware {
    Acpi(u64),
    DeviceTree(u64),
}

/// Semantic proposal only: intentionally has no repr(C), encoder or ABI version.
/// BootInfo v5 continues to use its existing production validator unchanged.
pub fn validate_future_boot(
    version: u32,
    firmware: Firmware,
    display: Option<(u32, u32, u32)>,
) -> Result<(), ContractError> {
    if version != 6 {
        return Err(ContractError::Version);
    }
    let address = match firmware {
        Firmware::Acpi(p) | Firmware::DeviceTree(p) => p,
    };
    if address == 0 || address % 8 != 0 {
        return Err(ContractError::Firmware);
    }
    if let Some((width, height, stride)) = display {
        if width == 0 || height == 0 || stride < width {
            return Err(ContractError::Display);
        }
    }
    Ok(())
}

/// Model of a proposed mapping request, not a page-table implementation.
pub fn validate_mapping(
    start: u64,
    bytes: u64,
    limit: u64,
    writable: bool,
    executable: bool,
) -> Result<(), ContractError> {
    if bytes == 0
        || !start.is_multiple_of(4096)
        || !bytes.is_multiple_of(4096)
        || start.checked_add(bytes).is_none_or(|end| end > limit)
    {
        return Err(ContractError::Address);
    }
    if writable && executable {
        return Err(ContractError::WritableExecutable);
    }
    Ok(())
}

pub fn validate_machine(architecture: Architecture, machine: u16) -> Result<(), ContractError> {
    if architecture.elf_machine() == machine {
        Ok(())
    } else {
        Err(ContractError::Machine)
    }
}

#[cfg(test)]
mod proposal_tests;

// Read actual production source; do not fork security semantics into a model.
// These host tests never execute privileged CPU instructions.
#[cfg(test)]
extern crate std;
#[cfg(test)]
#[allow(dead_code)] // Imported production APIs unused by this focused host harness.
#[path = "../../../kernel/src/acpi.rs"]
pub mod acpi;
#[cfg(test)]
#[allow(dead_code)] // Imported production APIs unused by this focused host harness.
#[path = "../../../kernel/src/cpu_tables.rs"]
pub mod cpu_tables;
#[cfg(test)]
#[allow(dead_code)] // Imported production APIs unused by this focused host harness.
#[path = "../../../kernel/src/handles.rs"]
pub mod handles;
#[cfg(test)]
#[allow(dead_code)] // Imported production APIs unused by this focused host harness.
#[path = "../../../kernel/src/ipc.rs"]
#[allow(clippy::unnecessary_mut_passed, clippy::drop_non_drop)]
// Baseline-only style lints; existing assertions are retained.
pub mod ipc;
#[cfg(test)]
#[allow(dead_code)] // Imported production APIs unused by this focused host harness.
#[path = "../../../loader/src/elf.rs"]
#[allow(clippy::derivable_impls)] // Baseline-only style lints; existing assertions are retained.
pub mod loader_elf;
#[cfg(test)]
#[allow(dead_code)] // Imported production APIs unused by this focused host harness.
#[path = "../../../kernel/src/memory.rs"]
pub mod memory;
#[cfg(test)]
#[allow(dead_code)] // Imported production APIs unused by this focused host harness.
#[path = "../../../kernel/src/process_exit.rs"]
pub mod process_exit;
#[cfg(test)]
#[allow(dead_code)] // Imported production APIs unused by this focused host harness.
#[path = "../../../kernel/src/scheduler.rs"]
#[allow(clippy::new_without_default)]
// Baseline-only style lints; existing assertions are retained.
pub mod scheduler;
#[cfg(test)]
#[allow(dead_code)] // Imported production APIs unused by this focused host harness.
#[path = "../../../kernel/src/user_elf.rs"]
pub mod user_elf;
#[cfg(test)]
#[allow(dead_code)] // Imported production APIs unused by this focused host harness.
#[path = "../../../kernel/src/vmo.rs"]
pub mod vmo;

#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../kernel/src/user_ipc.rs"]
pub mod user_ipc;
