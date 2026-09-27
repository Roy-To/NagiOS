pub const THREAD_SLOTS: usize = libnagi::BOOTSTRAP_USER_THREAD_COUNT;
pub const MIN_STACK_SIZE: usize = libnagi::BOOTSTRAP_USER_THREAD_STACK_MIN_SIZE;
pub const DEFAULT_STACK_SIZE: usize = libnagi::BOOTSTRAP_USER_THREAD_STACK_DEFAULT_SIZE;

pub fn thread_id_index(thread_id: u64) -> Option<usize> {
    let index = usize::try_from(thread_id).ok()?;
    (index < THREAD_SLOTS).then_some(index)
}

pub fn rounded_stack_size(requested: usize) -> Option<usize> {
    libnagi::round_bootstrap_user_thread_stack_size(requested)
}

#[cfg(test)]
mod tests {
    use super::{rounded_stack_size, thread_id_index, DEFAULT_STACK_SIZE, THREAD_SLOTS};

    #[test]
    fn thread_ids_are_bounded_without_aliasing() {
        assert_eq!(thread_id_index(0), Some(0));
        assert_eq!(thread_id_index(15), Some(15));
        assert_eq!(
            thread_id_index((THREAD_SLOTS - 1) as u64),
            Some(THREAD_SLOTS - 1)
        );
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
        assert_eq!(
            rounded_stack_size(DEFAULT_STACK_SIZE + 1),
            Some(DEFAULT_STACK_SIZE + 4096)
        );
        assert_eq!(rounded_stack_size(usize::MAX), None);
    }

    #[test]
    fn stack_sizes_accept_servo_script_thread_requirement() {
        let max_stack_size = libnagi::BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE;
        assert_eq!(max_stack_size, 8 * 1024 * 1024);
        assert_eq!(rounded_stack_size(max_stack_size), Some(max_stack_size));
        assert_eq!(rounded_stack_size(max_stack_size - 1), Some(max_stack_size));
        assert_eq!(rounded_stack_size(max_stack_size + 1), None);
    }
}
