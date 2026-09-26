//! Fixed-size diagnostics for the bounded Nagi pthread bridge.
//!
//! This formatter avoids allocation so it can report failures in the guest
//! allocator and thread bootstrap paths themselves.

const OUTPUT_CAPACITY: usize = 256;
const MAX_STAGE_LENGTH: usize = 64;

fn append_bytes(output: &mut [u8; OUTPUT_CAPACITY], offset: &mut usize, value: &[u8]) {
    let count = value.len().min(output.len().saturating_sub(*offset));
    output[*offset..*offset + count].copy_from_slice(&value[..count]);
    *offset += count;
}

fn append_number(output: &mut [u8; OUTPUT_CAPACITY], offset: &mut usize, value: usize) {
    let mut digits = [0u8; 20];
    let mut count = 0;
    let mut remaining = value;
    loop {
        digits[count] = b'0' + (remaining % 10) as u8;
        count += 1;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    while count > 0 && *offset < output.len() {
        count -= 1;
        output[*offset] = digits[count];
        *offset += 1;
    }
}

pub(crate) fn format_pthread_create_failure(
    attempt: usize,
    stage: &[u8],
    bridge_stack_bytes: usize,
    pthread_error: usize,
) -> ([u8; OUTPUT_CAPACITY], usize) {
    let mut output = [0u8; OUTPUT_CAPACITY];
    let mut offset = 0;
    append_bytes(
        &mut output,
        &mut offset,
        b"Nagi M17 trace: pthread_create failed attempt=",
    );
    append_number(&mut output, &mut offset, attempt);
    append_bytes(&mut output, &mut offset, b" stage=");
    append_bytes(
        &mut output,
        &mut offset,
        &stage[..stage.len().min(MAX_STAGE_LENGTH)],
    );
    append_bytes(&mut output, &mut offset, b" bridge_stack_bytes=");
    append_number(&mut output, &mut offset, bridge_stack_bytes);
    append_bytes(&mut output, &mut offset, b" pthread_error=");
    append_number(&mut output, &mut offset, pthread_error);
    append_bytes(&mut output, &mut offset, b"\r\n");
    (output, offset)
}

#[cfg(test)]
mod tests {
    use super::format_pthread_create_failure;

    #[test]
    fn formats_the_bridge_stack_and_returned_pthread_error() {
        let (line, length) =
            format_pthread_create_failure(12, b"native-thread-create-rejected", 16_384, 11);

        assert_eq!(
            &line[..length],
            b"Nagi M17 trace: pthread_create failed attempt=12 stage=native-thread-create-rejected bridge_stack_bytes=16384 pthread_error=11\r\n"
        );
    }

    #[test]
    fn bounds_long_stage_and_maximum_integer_fields() {
        let long_stage = [b'x'; 128];
        let (line, length) =
            format_pthread_create_failure(usize::MAX, &long_stage, usize::MAX, usize::MAX);

        assert!(length <= line.len());
        assert!(line[..length].ends_with(b" pthread_error=18446744073709551615\r\n"));
        assert_eq!(
            line[..length].iter().filter(|byte| **byte == b'x').count(),
            64
        );
    }
}
