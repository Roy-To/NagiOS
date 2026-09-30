pub const ARTIFACT_ID: &str = "nagi.m20.reader-fixture";
pub const FIXTURE_LEN: usize = 5_000;

pub const fn fixture_byte_at(offset: usize) -> u8 {
    if offset < 4 {
        return b"GGUF"[offset];
    }
    ((offset * 31 + offset / 7 + 0x5a) & 0xff) as u8
}

#[allow(dead_code)]
const fn build_fixture() -> [u8; FIXTURE_LEN] {
    let mut bytes = [0; FIXTURE_LEN];
    let mut offset = 0;
    while offset < FIXTURE_LEN {
        bytes[offset] = fixture_byte_at(offset);
        offset += 1;
    }
    bytes
}

#[allow(dead_code)]
pub const FIXTURE_BYTES: [u8; FIXTURE_LEN] = build_fixture();
