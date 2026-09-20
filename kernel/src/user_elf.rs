pub const USER_IMAGE_BASE: u64 = 0x0000_4000_0000_0000;
pub const USER_IMAGE_LIMIT: u64 = USER_IMAGE_BASE + 256 * 4096;
pub const MAX_LOAD_SEGMENTS: usize = 16;

const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const PAGE_SIZE: u64 = 4096;
const PT_LOAD: u32 = 1;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UserLoadSegment {
    pub file_offset: u64,
    pub virtual_address: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserLoadPlan {
    pub entry: u64,
    pub segments: [UserLoadSegment; MAX_LOAD_SEGMENTS],
    pub segment_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserElfError {
    Truncated,
    BadMagic,
    UnsupportedClass,
    UnsupportedEncoding,
    UnsupportedVersion,
    UnsupportedType,
    UnsupportedMachine,
    InvalidProgramHeaders,
    NoLoadSegments,
    TooManyLoadSegments,
    InvalidSegment,
    SegmentOutOfRange,
    WritableExecutable,
    OverlappingSegments,
    EntryNotExecutable,
}

pub fn parse(bytes: &[u8]) -> Result<UserLoadPlan, UserElfError> {
    if bytes.len() < ELF_HEADER_SIZE {
        return Err(UserElfError::Truncated);
    }
    if bytes.get(0..4) != Some(b"\x7fELF") {
        return Err(UserElfError::BadMagic);
    }
    if bytes[4] != 2 {
        return Err(UserElfError::UnsupportedClass);
    }
    if bytes[5] != 1 {
        return Err(UserElfError::UnsupportedEncoding);
    }
    if bytes[6] != 1 || read_u32(bytes, 20)? != 1 {
        return Err(UserElfError::UnsupportedVersion);
    }
    if read_u16(bytes, 16)? != 2 {
        return Err(UserElfError::UnsupportedType);
    }
    if read_u16(bytes, 18)? != 62 {
        return Err(UserElfError::UnsupportedMachine);
    }
    if read_u16(bytes, 52)? as usize != ELF_HEADER_SIZE
        || read_u16(bytes, 54)? as usize != PROGRAM_HEADER_SIZE
    {
        return Err(UserElfError::InvalidProgramHeaders);
    }

    let entry = read_u64(bytes, 24)?;
    let ph_offset =
        usize::try_from(read_u64(bytes, 32)?).map_err(|_| UserElfError::InvalidProgramHeaders)?;
    let ph_count = usize::from(read_u16(bytes, 56)?);
    let ph_bytes = ph_count
        .checked_mul(PROGRAM_HEADER_SIZE)
        .ok_or(UserElfError::InvalidProgramHeaders)?;
    let ph_end = ph_offset
        .checked_add(ph_bytes)
        .ok_or(UserElfError::InvalidProgramHeaders)?;
    if ph_count == 0 || ph_end > bytes.len() {
        return Err(UserElfError::InvalidProgramHeaders);
    }

    let mut load_count = 0_usize;
    for index in 0..ph_count {
        let offset = ph_offset + index * PROGRAM_HEADER_SIZE;
        if read_u32(bytes, offset)? == PT_LOAD {
            load_count += 1;
            if load_count > MAX_LOAD_SEGMENTS {
                return Err(UserElfError::TooManyLoadSegments);
            }
        }
    }

    let mut plan = UserLoadPlan {
        entry,
        segments: [UserLoadSegment::default(); MAX_LOAD_SEGMENTS],
        segment_count: 0,
    };
    for index in 0..ph_count {
        let offset = ph_offset + index * PROGRAM_HEADER_SIZE;
        if read_u32(bytes, offset)? != PT_LOAD {
            continue;
        }
        let segment = UserLoadSegment {
            flags: read_u32(bytes, offset + 4)?,
            file_offset: read_u64(bytes, offset + 8)?,
            virtual_address: read_u64(bytes, offset + 16)?,
            file_size: read_u64(bytes, offset + 32)?,
            memory_size: read_u64(bytes, offset + 40)?,
        };
        validate_segment(bytes, &plan, segment)?;
        plan.segments[plan.segment_count] = segment;
        plan.segment_count += 1;
    }
    if plan.segment_count == 0 {
        return Err(UserElfError::NoLoadSegments);
    }
    let entry_is_executable = plan.segments[..plan.segment_count].iter().any(|segment| {
        let Some(file_end) = segment.virtual_address.checked_add(segment.file_size) else {
            return false;
        };
        segment.flags & PF_X != 0 && entry >= segment.virtual_address && entry < file_end
    });
    if !entry_is_executable {
        return Err(UserElfError::EntryNotExecutable);
    }
    Ok(plan)
}

fn validate_segment(
    bytes: &[u8],
    plan: &UserLoadPlan,
    segment: UserLoadSegment,
) -> Result<(), UserElfError> {
    if segment.memory_size == 0
        || segment.file_size > segment.memory_size
        || !segment.virtual_address.is_multiple_of(PAGE_SIZE)
        || !segment.file_offset.is_multiple_of(PAGE_SIZE)
    {
        return Err(UserElfError::InvalidSegment);
    }
    if segment.flags & (PF_W | PF_X) == (PF_W | PF_X) {
        return Err(UserElfError::WritableExecutable);
    }
    let file_end = segment
        .file_offset
        .checked_add(segment.file_size)
        .ok_or(UserElfError::InvalidSegment)?;
    if file_end > bytes.len() as u64 {
        return Err(UserElfError::InvalidSegment);
    }
    let memory_end = segment
        .virtual_address
        .checked_add(segment.memory_size)
        .ok_or(UserElfError::SegmentOutOfRange)?;
    if segment.virtual_address < USER_IMAGE_BASE || memory_end > USER_IMAGE_LIMIT {
        return Err(UserElfError::SegmentOutOfRange);
    }
    for existing in &plan.segments[..plan.segment_count] {
        let existing_end = existing.virtual_address + existing.memory_size;
        if segment.virtual_address < existing_end && existing.virtual_address < memory_end {
            return Err(UserElfError::OverlappingSegments);
        }
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, UserElfError> {
    let raw: [u8; 2] = bytes
        .get(offset..offset + 2)
        .ok_or(UserElfError::Truncated)?
        .try_into()
        .map_err(|_| UserElfError::Truncated)?;
    Ok(u16::from_le_bytes(raw))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, UserElfError> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(UserElfError::Truncated)?
        .try_into()
        .map_err(|_| UserElfError::Truncated)?;
    Ok(u32::from_le_bytes(raw))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, UserElfError> {
    let raw: [u8; 8] = bytes
        .get(offset..offset + 8)
        .ok_or(UserElfError::Truncated)?
        .try_into()
        .map_err(|_| UserElfError::Truncated)?;
    Ok(u64::from_le_bytes(raw))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec;
    use std::vec::Vec;

    use super::{parse, UserElfError, USER_IMAGE_BASE, USER_IMAGE_LIMIT};

    const ELF_HEADER_SIZE: usize = 64;
    const PROGRAM_HEADER_SIZE: usize = 56;
    const SEGMENT_OFFSET: usize = 0x1000;

    fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn elf_with_segment(virtual_address: u64, memory_size: u64, flags: u32) -> Vec<u8> {
        let file_size = 16_u64;
        let mut bytes = vec![0_u8; SEGMENT_OFFSET + file_size as usize];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[6] = 1;
        write_u16(&mut bytes, 16, 2);
        write_u16(&mut bytes, 18, 62);
        write_u32(&mut bytes, 20, 1);
        write_u64(&mut bytes, 24, virtual_address);
        write_u64(&mut bytes, 32, ELF_HEADER_SIZE as u64);
        write_u16(&mut bytes, 52, ELF_HEADER_SIZE as u16);
        write_u16(&mut bytes, 54, PROGRAM_HEADER_SIZE as u16);
        write_u16(&mut bytes, 56, 1);

        let ph = ELF_HEADER_SIZE;
        write_u32(&mut bytes, ph, 1);
        write_u32(&mut bytes, ph + 4, flags);
        write_u64(&mut bytes, ph + 8, SEGMENT_OFFSET as u64);
        write_u64(&mut bytes, ph + 16, virtual_address);
        write_u64(&mut bytes, ph + 24, virtual_address);
        write_u64(&mut bytes, ph + 32, file_size);
        write_u64(&mut bytes, ph + 40, memory_size);
        write_u64(&mut bytes, ph + 48, 4096);
        bytes[SEGMENT_OFFSET..SEGMENT_OFFSET + file_size as usize].fill(0x90);
        bytes
    }

    #[test]
    fn parses_a_bounded_executable_user_segment() {
        let bytes = elf_with_segment(USER_IMAGE_BASE, 4096, 5);

        let plan = parse(&bytes).expect("valid user ELF");

        assert_eq!(plan.entry, USER_IMAGE_BASE);
        assert_eq!(plan.segment_count, 1);
        assert_eq!(plan.segments[0].virtual_address, USER_IMAGE_BASE);
        assert_eq!(plan.segments[0].file_offset, SEGMENT_OFFSET as u64);
    }

    #[test]
    fn rejects_identity_only_addresses() {
        let bytes = elf_with_segment(0x20_0000, 4096, 5);
        assert_eq!(parse(&bytes), Err(UserElfError::SegmentOutOfRange));
    }

    #[test]
    fn rejects_writable_executable_segments() {
        let bytes = elf_with_segment(USER_IMAGE_BASE, 4096, 7);
        assert_eq!(parse(&bytes), Err(UserElfError::WritableExecutable));
    }

    #[test]
    fn rejects_segments_outside_the_bounded_image_range() {
        let bytes = elf_with_segment(USER_IMAGE_LIMIT, 4096, 5);
        assert_eq!(parse(&bytes), Err(UserElfError::SegmentOutOfRange));
    }

    #[test]
    fn rejects_entry_outside_executable_file_bytes() {
        let mut bytes = elf_with_segment(USER_IMAGE_BASE, 4096, 5);
        write_u64(&mut bytes, 24, USER_IMAGE_BASE + 16);
        assert_eq!(parse(&bytes), Err(UserElfError::EntryNotExecutable));
    }

    #[test]
    fn rejects_overlapping_load_segments() {
        let mut bytes = elf_with_segment(USER_IMAGE_BASE, 4096, 5);
        bytes.resize(SEGMENT_OFFSET + 0x1010, 0);
        write_u16(&mut bytes, 56, 2);
        let first = bytes[ELF_HEADER_SIZE..ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE].to_vec();
        bytes[ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE..ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE * 2]
            .copy_from_slice(&first);
        let second = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE;
        write_u64(&mut bytes, second + 8, (SEGMENT_OFFSET + 0x1000) as u64);
        write_u64(&mut bytes, second + 16, USER_IMAGE_BASE);
        write_u64(&mut bytes, second + 24, USER_IMAGE_BASE);

        assert_eq!(parse(&bytes), Err(UserElfError::OverlappingSegments));
    }

    #[test]
    fn rejects_more_than_sixteen_load_segments_before_segment_validation() {
        let mut bytes = elf_with_segment(USER_IMAGE_BASE, 4096, 5);
        write_u16(&mut bytes, 56, 17);
        let first = bytes[ELF_HEADER_SIZE..ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE].to_vec();
        for index in 1..17 {
            let start = ELF_HEADER_SIZE + index * PROGRAM_HEADER_SIZE;
            bytes[start..start + PROGRAM_HEADER_SIZE].copy_from_slice(&first);
        }

        assert_eq!(parse(&bytes), Err(UserElfError::TooManyLoadSegments));
    }
}
