use core::convert::TryFrom;

const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u8 = 1;
const EM_X86_64: u16 = 0x3E;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PAGE_SIZE: u64 = 4096;
pub const MAX_LOAD_SEGMENTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadSegment {
    pub file_offset: u64,
    pub physical_address: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub flags: u32,
}

impl Default for LoadSegment {
    fn default() -> Self {
        Self {
            file_offset: 0,
            physical_address: 0,
            file_size: 0,
            memory_size: 0,
            flags: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadPlan {
    pub entry: u64,
    pub segments: [LoadSegment; MAX_LOAD_SEGMENTS],
    pub segment_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElfError {
    Truncated,
    BadMagic,
    UnsupportedClass,
    UnsupportedEncoding,
    UnsupportedMachine,
    UnsupportedHeader,
    ProgramHeadersOutOfBounds,
    TooManyLoadSegments,
    NoLoadSegments,
    SegmentOutOfBounds,
    SegmentSizeInvalid,
    SegmentAddressOverflow,
    SegmentNotIdentityMapped,
    SegmentNotPageAligned,
    SegmentAlignmentInvalid,
    SegmentOverlap,
    EntryNotLoaded,
}

pub fn parse(bytes: &[u8]) -> Result<LoadPlan, ElfError> {
    if bytes.len() < ELF_HEADER_SIZE {
        return Err(ElfError::Truncated);
    }
    if &bytes[..4] != b"\x7FELF" {
        return Err(ElfError::BadMagic);
    }
    if bytes[4] != ELFCLASS64 {
        return Err(ElfError::UnsupportedClass);
    }
    if bytes[5] != ELFDATA2LSB || bytes[6] != EV_CURRENT {
        return Err(ElfError::UnsupportedEncoding);
    }
    if read_u16(bytes, 18)? != EM_X86_64 {
        return Err(ElfError::UnsupportedMachine);
    }
    if read_u32(bytes, 20)? != 1
        || read_u16(bytes, 52)? as usize != ELF_HEADER_SIZE
        || read_u16(bytes, 54)? as usize != PROGRAM_HEADER_SIZE
    {
        return Err(ElfError::UnsupportedHeader);
    }

    let entry = read_u64(bytes, 24)?;
    let program_header_offset =
        usize::try_from(read_u64(bytes, 32)?).map_err(|_| ElfError::ProgramHeadersOutOfBounds)?;
    let program_header_count = read_u16(bytes, 56)? as usize;
    let program_headers_size = program_header_count
        .checked_mul(PROGRAM_HEADER_SIZE)
        .ok_or(ElfError::ProgramHeadersOutOfBounds)?;
    let program_headers_end = program_header_offset
        .checked_add(program_headers_size)
        .ok_or(ElfError::ProgramHeadersOutOfBounds)?;
    if program_headers_end > bytes.len() {
        return Err(ElfError::ProgramHeadersOutOfBounds);
    }

    let mut plan = LoadPlan {
        entry,
        segments: [LoadSegment::default(); MAX_LOAD_SEGMENTS],
        segment_count: 0,
    };
    for index in 0..program_header_count {
        let offset = program_header_offset + index * PROGRAM_HEADER_SIZE;
        if read_u32(bytes, offset)? != PT_LOAD {
            continue;
        }
        if plan.segment_count == MAX_LOAD_SEGMENTS {
            return Err(ElfError::TooManyLoadSegments);
        }
        let flags = read_u32(bytes, offset + 4)?;
        let file_offset = read_u64(bytes, offset + 8)?;
        let virtual_address = read_u64(bytes, offset + 16)?;
        let physical_address = read_u64(bytes, offset + 24)?;
        let file_size = read_u64(bytes, offset + 32)?;
        let memory_size = read_u64(bytes, offset + 40)?;
        let alignment = read_u64(bytes, offset + 48)?;

        if memory_size < file_size {
            return Err(ElfError::SegmentSizeInvalid);
        }
        if virtual_address != physical_address {
            return Err(ElfError::SegmentNotIdentityMapped);
        }
        if physical_address % PAGE_SIZE != 0 {
            return Err(ElfError::SegmentNotPageAligned);
        }
        if alignment != 0 && (!alignment.is_power_of_two() || alignment < PAGE_SIZE) {
            return Err(ElfError::SegmentAlignmentInvalid);
        }
        let file_end = file_offset
            .checked_add(file_size)
            .ok_or(ElfError::SegmentOutOfBounds)?;
        if file_end > bytes.len() as u64 {
            return Err(ElfError::SegmentOutOfBounds);
        }
        physical_address
            .checked_add(memory_size)
            .ok_or(ElfError::SegmentAddressOverflow)?;
        let segment = LoadSegment {
            file_offset,
            physical_address,
            file_size,
            memory_size,
            flags,
        };
        for previous in &plan.segments[..plan.segment_count] {
            let previous_end = previous
                .physical_address
                .checked_add(previous.memory_size)
                .ok_or(ElfError::SegmentAddressOverflow)?;
            if physical_address < previous_end
                && previous.physical_address
                    < physical_address
                        .checked_add(memory_size)
                        .ok_or(ElfError::SegmentAddressOverflow)?
            {
                return Err(ElfError::SegmentOverlap);
            }
        }
        plan.segments[plan.segment_count] = segment;
        plan.segment_count += 1;
    }

    if plan.segment_count == 0 {
        return Err(ElfError::NoLoadSegments);
    }
    if !plan.segments[..plan.segment_count].iter().any(|segment| {
        segment.flags & PF_X != 0
            && entry >= segment.physical_address
            && entry < segment.physical_address + segment.memory_size
    }) {
        return Err(ElfError::EntryNotLoaded);
    }
    Ok(plan)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ElfError> {
    let end = offset.checked_add(2).ok_or(ElfError::Truncated)?;
    let bytes = bytes.get(offset..end).ok_or(ElfError::Truncated)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ElfError> {
    let end = offset.checked_add(4).ok_or(ElfError::Truncated)?;
    let bytes = bytes.get(offset..end).ok_or(ElfError::Truncated)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, ElfError> {
    let end = offset.checked_add(8).ok_or(ElfError::Truncated)?;
    let bytes = bytes.get(offset..end).ok_or(ElfError::Truncated)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec;
    use std::vec::Vec;

    use super::*;

    fn valid_elf() -> Vec<u8> {
        let mut bytes = vec![0; 64 + 56 + 16];
        bytes[..4].copy_from_slice(b"\x7FELF");
        bytes[4] = ELFCLASS64;
        bytes[5] = ELFDATA2LSB;
        bytes[6] = EV_CURRENT;
        bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&0x0020_0000u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[52..54].copy_from_slice(&(ELF_HEADER_SIZE as u16).to_le_bytes());
        bytes[54..56].copy_from_slice(&(PROGRAM_HEADER_SIZE as u16).to_le_bytes());
        bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
        let ph = 64;
        bytes[ph..ph + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
        bytes[ph + 4..ph + 8].copy_from_slice(&PF_X.to_le_bytes());
        bytes[ph + 8..ph + 16].copy_from_slice(&120u64.to_le_bytes());
        bytes[ph + 16..ph + 24].copy_from_slice(&0x0020_0000u64.to_le_bytes());
        bytes[ph + 24..ph + 32].copy_from_slice(&0x0020_0000u64.to_le_bytes());
        bytes[ph + 32..ph + 40].copy_from_slice(&16u64.to_le_bytes());
        bytes[ph + 40..ph + 48].copy_from_slice(&4096u64.to_le_bytes());
        bytes[ph + 48..ph + 56].copy_from_slice(&4096u64.to_le_bytes());
        bytes[120..136].fill(0x90);
        bytes
    }

    #[test]
    fn parses_a_valid_identity_mapped_kernel() {
        let plan = parse(&valid_elf()).expect("valid ELF");
        assert_eq!(plan.entry, 0x0020_0000);
        assert_eq!(plan.segment_count, 1);
        assert_eq!(plan.segments[0].memory_size, 4096);
    }

    #[test]
    fn rejects_truncated_headers() {
        assert_eq!(parse(&[0x7F, b'E', b'L', b'F']), Err(ElfError::Truncated));
    }

    #[test]
    fn rejects_non_identity_segments() {
        let mut bytes = valid_elf();
        bytes[64 + 16..64 + 24].copy_from_slice(&0xFFFF_8000_0020_0000u64.to_le_bytes());
        assert_eq!(parse(&bytes), Err(ElfError::SegmentNotIdentityMapped));
    }

    #[test]
    fn rejects_segment_file_bounds() {
        let mut bytes = valid_elf();
        bytes[64 + 32..64 + 40].copy_from_slice(&0x1000u64.to_le_bytes());
        assert_eq!(parse(&bytes), Err(ElfError::SegmentOutOfBounds));
    }
}
