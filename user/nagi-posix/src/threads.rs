pub const THREAD_SLOTS: usize = libnagi::BOOTSTRAP_USER_THREAD_COUNT;
pub const MIN_STACK_SIZE: usize = 4096;
pub const DEFAULT_STACK_SIZE: usize = 2 * 1024 * 1024;

pub fn thread_id_index(thread_id: u64) -> Option<usize> {
    let index = usize::try_from(thread_id).ok()?;
    (index < THREAD_SLOTS).then_some(index)
}

pub fn rounded_stack_size(requested: usize) -> Option<usize> {
    let size = if requested == 0 {
        DEFAULT_STACK_SIZE
    } else {
        requested
    };
    if !(MIN_STACK_SIZE..=DEFAULT_STACK_SIZE).contains(&size) {
        return None;
    }
    let rounded = size.checked_add(4095)? & !4095;
    (rounded <= DEFAULT_STACK_SIZE).then_some(rounded)
}

#[cfg(test)]
mod tests {
    use super::{rounded_stack_size, thread_id_index, DEFAULT_STACK_SIZE, THREAD_SLOTS};

    #[test]
    fn thread_ids_are_bounded_without_aliasing() {
        assert_eq!(thread_id_index(0), Some(0));
        assert_eq!(thread_id_index(15), Some(15));
        assert_eq!(thread_id_index(THREAD_SLOTS as u64), None);
        assert_eq!(thread_id_index(u64::MAX), None);
    }

    #[test]
    fn stack_sizes_use_default_minimum_and_page_rounding() {
        assert_eq!(rounded_stack_size(0), Some(DEFAULT_STACK_SIZE));
        assert_eq!(rounded_stack_size(4096), Some(4096));
        assert_eq!(rounded_stack_size(4097), Some(8192));
        assert_eq!(
            rounded_stack_size(DEFAULT_STACK_SIZE),
            Some(DEFAULT_STACK_SIZE)
        );
        assert_eq!(rounded_stack_size(1), None);
        assert_eq!(rounded_stack_size(DEFAULT_STACK_SIZE + 1), None);
        assert_eq!(rounded_stack_size(usize::MAX), None);
    }
}
