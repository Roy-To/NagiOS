pub const BLOCKED: u32 = 0;
pub const RUNNABLE: u32 = 1;
pub const RUNNING: u32 = 2;
pub const DONE: u32 = 3;

pub fn wake_transition(state: u32) -> Option<u32> {
    (state == BLOCKED).then_some(RUNNABLE)
}

#[cfg(test)]
mod tests {
    use super::{wake_transition, BLOCKED, DONE, RUNNABLE, RUNNING};

    #[test]
    fn only_blocked_threads_can_be_woken() {
        assert_eq!(wake_transition(BLOCKED), Some(RUNNABLE));
        assert_eq!(wake_transition(RUNNABLE), None);
        assert_eq!(wake_transition(RUNNING), None);
        assert_eq!(wake_transition(DONE), None);
    }
}
