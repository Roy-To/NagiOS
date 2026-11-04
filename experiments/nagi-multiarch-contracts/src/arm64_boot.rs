//! Host-only preparation proposal. No MMIO, pointer dereference or boot backend.
//! Passing this envelope check does not validate DT properties or authenticate
//! firmware. Physical platforms remain unsupported even with complete inputs.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    QemuVirt,
    Physical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    UnsupportedPlatform,
    MissingFirmwareIdentity,
    MissingDeviceTree,
    DeviceTreeEnvelope,
    ExceptionLevel,
    MmuEnabled,
    Uart,
}

/// Proposed preparation facts supplied by a caller, not discovered hardware.
pub struct Inputs<'a> {
    pub platform: Platform,
    pub firmware_identity: Option<&'a str>,
    pub device_tree: Option<&'a [u8]>,
    pub current_el: Option<u64>,
    pub sctlr_el1: Option<u64>,
    pub uart_base: Option<u64>,
}

/// Validate only the bounded FDT v17 envelope of a captured byte slice.
/// Do not follow the original firmware pointer or infer any device resources.
pub fn validate_dtb_envelope(data: &[u8]) -> Result<(), Error> {
    let invalid = Error::DeviceTreeEnvelope;
    if data.len() < 40 {
        return Err(invalid);
    }
    let word = |offset| u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
    let total = word(4);
    if word(0) != 0xd00d_feed
        || total < 40
        || total > data.len()
        || word(20) != 17
        || word(24) != 16
    {
        return Err(invalid);
    }
    let structure = word(8);
    let strings = word(12);
    let reservations = word(16);
    let structure_end = structure.checked_add(word(36)).ok_or(invalid)?;
    let strings_end = strings.checked_add(word(32)).ok_or(invalid)?;
    if structure < 40
        || structure % 4 != 0
        || word(36) == 0
        || strings < 40
        || reservations < 40
        || reservations % 8 != 0
        || structure_end > total
        || strings_end > total
        || reservations.checked_add(16).is_none_or(|end| end > total)
        || reservations >= structure
        || structure_end > strings
    {
        return Err(invalid);
    }
    // A terminating reservation pair must fit before either following block.
    let reservation_limit = [structure, strings, total]
        .into_iter()
        .filter(|offset| *offset >= reservations)
        .min()
        .ok_or(invalid)?;
    let mut cursor = reservations;
    loop {
        let end = cursor.checked_add(16).ok_or(invalid)?;
        if end > reservation_limit {
            return Err(invalid);
        }
        if data[cursor..end].iter().all(|byte| *byte == 0) {
            break;
        }
        cursor = end;
    }
    Ok(())
}

/// Bounded QEMU-virt preparation gate; deliberately not wired into production.
pub fn validate(inputs: &Inputs<'_>) -> Result<(), Error> {
    if inputs.platform != Platform::QemuVirt {
        return Err(Error::UnsupportedPlatform);
    }
    if inputs
        .firmware_identity
        .is_none_or(|identity| identity.trim().is_empty())
    {
        return Err(Error::MissingFirmwareIdentity);
    }
    validate_dtb_envelope(inputs.device_tree.ok_or(Error::MissingDeviceTree)?)?;
    // CurrentEL is the architectural register encoding, not the integer EL.
    if inputs.current_el != Some(4) {
        return Err(Error::ExceptionLevel);
    }
    if inputs.sctlr_el1.is_none_or(|value| value & 1 != 0) {
        return Err(Error::MmuEnabled);
    }
    if inputs.uart_base != Some(0x0900_0000) {
        return Err(Error::Uart);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dtb() -> [u8; 64] {
        // Synthetic envelope only: deliberately not a complete DT property tree.
        let mut bytes = [0; 64];
        for (offset, value) in [
            (0, 0xd00d_feedu32),
            (4, 64),
            (8, 56),
            (12, 60),
            (16, 40),
            (20, 17),
            (24, 16),
            (32, 4),
            (36, 4),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        bytes
    }

    fn inputs(tree: &[u8]) -> Inputs<'_> {
        Inputs {
            platform: Platform::QemuVirt,
            firmware_identity: Some("synthetic fixture"),
            device_tree: Some(tree),
            current_el: Some(4),
            sctlr_el1: Some(0),
            uart_base: Some(0x0900_0000),
        }
    }

    #[test]
    fn missing_public_facts_do_not_fall_back_to_qemu_defaults() {
        let tree = dtb();
        let mut facts = inputs(&tree);
        assert_eq!(validate(&facts), Ok(()));
        facts.firmware_identity = None;
        assert_eq!(validate(&facts), Err(Error::MissingFirmwareIdentity));
        facts.firmware_identity = Some("  ");
        assert_eq!(validate(&facts), Err(Error::MissingFirmwareIdentity));
        facts.firmware_identity = Some("fixture");
        facts.device_tree = None;
        assert_eq!(validate(&facts), Err(Error::MissingDeviceTree));
    }

    #[test]
    fn physical_is_unsupported_even_with_qemu_shaped_facts() {
        let tree = dtb();
        let mut facts = inputs(&tree);
        facts.platform = Platform::Physical;
        assert_eq!(validate(&facts), Err(Error::UnsupportedPlatform));
    }

    #[test]
    fn exception_level_mmu_and_uart_boundaries() {
        let tree = dtb();
        let mut facts = inputs(&tree);
        for el in [None, Some(0), Some(1), Some(8), Some(12), Some(5)] {
            facts.current_el = el;
            assert_eq!(validate(&facts), Err(Error::ExceptionLevel));
        }
        facts.current_el = Some(4);
        for control in [None, Some(1), Some(u64::MAX)] {
            facts.sctlr_el1 = control;
            assert_eq!(validate(&facts), Err(Error::MmuEnabled));
        }
        facts.sctlr_el1 = Some(0);
        for uart in [None, Some(0), Some(0xfe20_1000)] {
            facts.uart_base = uart;
            assert_eq!(validate(&facts), Err(Error::Uart));
        }
    }

    #[test]
    fn every_header_truncation_is_rejected_without_dereferencing() {
        let tree = dtb();
        for end in 0..tree.len() {
            assert_eq!(
                validate_dtb_envelope(&tree[..end]),
                Err(Error::DeviceTreeEnvelope)
            );
        }
    }

    #[test]
    fn malformed_big_endian_ranges_and_versions_are_rejected() {
        for (offset, value) in [
            (0, 0),
            (4, 39),
            (4, 65),
            (8, 57),
            (8, u32::MAX),
            (12, 56),
            (16, 41),
            (16, 56),
            (20, 16),
            (24, 18),
            (32, u32::MAX),
            (36, 0),
            (36, u32::MAX),
        ] {
            let mut tree = dtb();
            tree[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
            assert_eq!(
                validate_dtb_envelope(&tree),
                Err(Error::DeviceTreeEnvelope),
                "offset {offset}, value {value}"
            );
        }
        let mut tree = dtb();
        tree[40] = 1; // Nonterminating reservation overlaps structure block.
        assert_eq!(validate_dtb_envelope(&tree), Err(Error::DeviceTreeEnvelope));
    }
}
