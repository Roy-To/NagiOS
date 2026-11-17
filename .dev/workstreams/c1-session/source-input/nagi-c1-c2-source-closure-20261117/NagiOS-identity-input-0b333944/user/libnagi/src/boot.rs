#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootMode {
    Simulation,
    External,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootPhase {
    SystemInit,
    CoreServices,
    StorageMount,
    GraphicsReady,
    SessionReady,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootStage {
    Platform,
    CoreServices,
    Storage,
    Graphics,
    Session,
}

impl BootStage {
    pub const fn target(self) -> u8 {
        match self {
            Self::Platform => 15,
            Self::CoreServices => 30,
            Self::Storage => 50,
            Self::Graphics => 70,
            Self::Session => 90,
        }
    }

    pub const fn phase(self) -> BootPhase {
        match self {
            Self::Platform => BootPhase::SystemInit,
            Self::CoreServices => BootPhase::CoreServices,
            Self::Storage => BootPhase::StorageMount,
            Self::Graphics => BootPhase::GraphicsReady,
            Self::Session => BootPhase::SessionReady,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootState {
    mode: BootMode,
    reduced_motion: bool,
    progress: u8,
    phase: BootPhase,
    lock_ready: bool,
    completed: bool,
    failed: bool,
}

impl BootState {
    pub const fn mode(&self) -> BootMode {
        self.mode
    }

    pub const fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    pub const fn progress(&self) -> u8 {
        self.progress
    }

    pub const fn phase(&self) -> BootPhase {
        self.phase
    }

    pub const fn lock_ready(&self) -> bool {
        self.lock_ready
    }

    pub const fn completed(&self) -> bool {
        self.completed
    }

    pub const fn failed(&self) -> bool {
        self.failed
    }
}

pub struct BootProgressBridge {
    state: BootState,
}

impl BootProgressBridge {
    pub const fn new(mode: BootMode) -> Self {
        Self {
            state: BootState {
                mode,
                reduced_motion: false,
                progress: 0,
                phase: BootPhase::SystemInit,
                lock_ready: false,
                completed: false,
                failed: false,
            },
        }
    }

    pub fn set_mode(&mut self, mode: BootMode) {
        self.state.mode = mode;
    }

    pub fn set_reduced_motion(&mut self, reduced_motion: bool) {
        self.state.reduced_motion = reduced_motion;
    }

    pub fn set_progress(&mut self, requested: u8, phase: BootPhase) -> bool {
        if self.state.failed() || self.state.completed() {
            return false;
        }
        let next = requested.min(100).max(self.state.progress());
        let next = if !self.state.lock_ready() {
            next.min(99)
        } else {
            next
        };
        let changed = next != self.state.progress();
        if changed {
            self.state.progress = next;
            self.state.phase = phase;
        }
        changed
    }

    pub fn advance(&mut self, stage: BootStage) -> bool {
        self.set_progress(stage.target(), stage.phase())
    }

    pub fn mark_lock_ready(&mut self) -> bool {
        if self.state.failed() || self.state.completed() || self.state.lock_ready() {
            return false;
        }
        self.state.lock_ready = true;
        true
    }

    pub fn complete(&mut self) -> bool {
        if self.state.failed() || !self.state.lock_ready() {
            return false;
        }
        self.state.progress = 100;
        self.state.phase = BootPhase::Ready;
        self.state.completed = true;
        true
    }

    pub fn fail(&mut self) -> bool {
        if self.state.failed() || self.state.completed() {
            return false;
        }
        self.state.phase = BootPhase::Failed;
        self.state.failed = true;
        true
    }

    pub const fn state(&self) -> BootState {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_motion_defaults_off_and_can_be_set() {
        let mut bridge = BootProgressBridge::new(BootMode::External);
        assert!(!bridge.state().reduced_motion());

        bridge.set_reduced_motion(true);
        assert!(bridge.state().reduced_motion());

        bridge.set_reduced_motion(false);
        assert!(!bridge.state().reduced_motion());
    }

    #[test]
    fn progress_never_moves_backward() {
        let mut bridge = BootProgressBridge::new(BootMode::External);
        assert!(bridge.set_progress(70, BootPhase::GraphicsReady));
        assert!(!bridge.set_progress(30, BootPhase::CoreServices));
        assert_eq!(bridge.state().progress(), 70);
        assert_eq!(bridge.state().phase(), BootPhase::GraphicsReady);
    }

    #[test]
    fn progress_caps_at_ninety_nine_until_lock_is_ready() {
        let mut bridge = BootProgressBridge::new(BootMode::External);
        assert!(bridge.set_progress(100, BootPhase::Ready));
        assert_eq!(bridge.state().progress(), 99);
        assert!(!bridge.state().completed());
        assert!(bridge.mark_lock_ready());
        assert!(bridge.complete());
        assert_eq!(bridge.state().progress(), 100);
        assert!(bridge.state().completed());
    }

    #[test]
    fn stage_targets_are_monotonic_and_named() {
        let mut bridge = BootProgressBridge::new(BootMode::External);
        assert!(bridge.advance(BootStage::Platform));
        assert_eq!(bridge.state().progress(), 15);
        assert_eq!(bridge.state().phase(), BootPhase::SystemInit);
        assert!(bridge.advance(BootStage::CoreServices));
        assert_eq!(bridge.state().progress(), 30);
        assert_eq!(bridge.state().phase(), BootPhase::CoreServices);
        assert!(bridge.advance(BootStage::Storage));
        assert_eq!(bridge.state().progress(), 50);
        assert_eq!(bridge.state().phase(), BootPhase::StorageMount);
    }

    #[test]
    fn failure_is_terminal_and_does_not_complete() {
        let mut bridge = BootProgressBridge::new(BootMode::External);
        assert!(bridge.advance(BootStage::Storage));
        assert!(bridge.fail());
        assert!(bridge.state().failed());
        assert_eq!(bridge.state().phase(), BootPhase::Failed);
        assert!(!bridge.complete());
    }
}
