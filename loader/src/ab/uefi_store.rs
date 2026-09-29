//! UEFI non-volatile variable adapter for the boot-control journal.

use super::{BootControlStore, StoreError, RECORD_SIZE};
use uefi::runtime::{self, VariableAttributes, VariableVendor};
use uefi::{cstr16, guid};

const VENDOR: VariableVendor = VariableVendor(guid!("c00c1ca4-6d7c-4a36-b913-a22f691c4895"));
const REQUIRED_ATTRIBUTES: VariableAttributes = VariableAttributes::NON_VOLATILE
    .union(VariableAttributes::BOOTSERVICE_ACCESS)
    .union(VariableAttributes::RUNTIME_ACCESS);

/// Store the journal's two fixed-size copies in separate firmware variables.
///
/// `set_variable` is called for every write, so this adapter does not buffer
/// data and `flush` is a no-op after a successful firmware write.
pub struct UefiVariableBootControlStore;

impl UefiVariableBootControlStore {
    pub const fn new() -> Self {
        Self
    }

    fn name(copy: usize) -> Result<&'static uefi::CStr16, StoreError> {
        match copy {
            0 => Ok(cstr16!("NagiBootControl0")),
            1 => Ok(cstr16!("NagiBootControl1")),
            _ => Err(StoreError::Io),
        }
    }
}

impl Default for UefiVariableBootControlStore {
    fn default() -> Self {
        Self::new()
    }
}

impl BootControlStore for UefiVariableBootControlStore {
    fn read_record(
        &mut self,
        copy: usize,
        output: &mut [u8; RECORD_SIZE],
    ) -> Result<Option<usize>, StoreError> {
        let name = Self::name(copy)?;
        if !runtime::variable_exists(name, &VENDOR).map_err(|_| StoreError::Io)? {
            return Ok(None);
        }

        let (record, attributes) =
            runtime::get_variable(name, &VENDOR, output).map_err(|_| StoreError::Io)?;
        if !attributes.contains(REQUIRED_ATTRIBUTES) {
            return Err(StoreError::Io);
        }
        Ok(Some(record.len()))
    }

    fn write_record(&mut self, copy: usize, record: &[u8; RECORD_SIZE]) -> Result<(), StoreError> {
        runtime::set_variable(Self::name(copy)?, &VENDOR, REQUIRED_ATTRIBUTES, record)
            .map_err(|_| StoreError::Io)
    }

    fn flush(&mut self) -> Result<(), StoreError> {
        Ok(())
    }
}
