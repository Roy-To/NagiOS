#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpawnError {
    Unsupported,
    InvalidRequest,
    CapabilityEscalation,
}

/// Native Nagi spawn request. `requested_rights` is an attenuation mask over
/// the rights explicitly held by the caller; it is never a path or a host
/// process command. The bounded bootstrap implementation accepts one child
/// entry at a time and waits for its result through a Nagi thread/process
/// primitive.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NagiSpawnRequest {
    pub entry: usize,
    pub argument: usize,
    pub parent_rights: u64,
    pub requested_rights: u64,
}

#[inline]
pub fn attenuate_rights(parent_rights: u64, requested_rights: u64) -> Option<u64> {
    (requested_rights & !parent_rights == 0).then_some(requested_rights)
}

#[cfg(target_os = "nagi")]
type NativeSpawnEntry = extern "C" fn(usize, u64) -> u64;

#[cfg(target_os = "nagi")]
#[repr(C)]
struct NativeSpawnRecord {
    entry: NativeSpawnEntry,
    argument: usize,
    rights: u64,
}

#[cfg(target_os = "nagi")]
unsafe impl Sync for NativeSpawnRecord {}

#[cfg(target_os = "nagi")]
static mut NATIVE_SPAWN_RECORD: Option<NativeSpawnRecord> = None;
#[cfg(target_os = "nagi")]
static mut NATIVE_SPAWN_STACK: *mut u8 = core::ptr::null_mut();
#[cfg(target_os = "nagi")]
static mut NATIVE_SPAWN_THREAD: Option<u64> = None;

#[cfg(target_os = "nagi")]
extern "C" fn native_spawn_trampoline(record: *mut core::ffi::c_void) -> ! {
    let record = unsafe {
        (&*record.cast::<Option<NativeSpawnRecord>>())
            .as_ref()
            .expect("native spawn record")
    };
    let result = (record.entry)(record.argument, record.rights);
    libnagi::thread_exit(result)
}

/// Nagi process creation is spawn-oriented. Until the process service is
/// available, the C ABI reports this explicitly rather than emulating fork.
pub fn posix_spawn(_program: &[u8], _arguments: &[&[u8]]) -> Result<u64, SpawnError> {
    Err(SpawnError::Unsupported)
}

#[cfg(target_os = "nagi")]
pub unsafe fn native_spawn(request: &NagiSpawnRequest) -> Result<u64, SpawnError> {
    let active_spawn = unsafe { NATIVE_SPAWN_THREAD };
    if active_spawn.is_some() {
        return Err(SpawnError::InvalidRequest);
    }
    if request.entry == 0 {
        return Err(SpawnError::InvalidRequest);
    }
    let Some(rights) = attenuate_rights(request.parent_rights, request.requested_rights) else {
        return Err(SpawnError::CapabilityEscalation);
    };
    let Some(entry) = core::num::NonZeroUsize::new(request.entry) else {
        return Err(SpawnError::InvalidRequest);
    };
    let entry = core::mem::transmute::<usize, NativeSpawnEntry>(entry.get());
    let stack_protection = (libnagi::PROT_READ | libnagi::PROT_WRITE) as i32;
    let stack_size = crate::threads::DEFAULT_STACK_SIZE;
    let stack = crate::nagi_posix_mmap(stack_size, stack_protection);
    if stack.is_null() {
        return Err(SpawnError::InvalidRequest);
    }
    NATIVE_SPAWN_RECORD = Some(NativeSpawnRecord {
        entry,
        argument: request.argument,
        rights,
    });
    let record = core::ptr::addr_of_mut!(NATIVE_SPAWN_RECORD) as *mut core::ffi::c_void;
    let Some(thread) = libnagi::thread_create(
        native_spawn_trampoline as usize,
        record as usize,
        stack,
        stack_size,
    ) else {
        let _ = crate::nagi_posix_munmap(stack, stack_size);
        NATIVE_SPAWN_RECORD = None;
        return Err(SpawnError::InvalidRequest);
    };
    NATIVE_SPAWN_STACK = stack;
    NATIVE_SPAWN_THREAD = Some(thread);
    Ok(thread)
}

#[cfg(target_os = "nagi")]
pub unsafe fn native_wait(thread: u64) -> Result<u64, SpawnError> {
    if NATIVE_SPAWN_THREAD != Some(thread) {
        return Err(SpawnError::InvalidRequest);
    }
    let Some(result) = libnagi::thread_join(thread) else {
        return Err(SpawnError::InvalidRequest);
    };
    if !NATIVE_SPAWN_STACK.is_null() {
        let stack = NATIVE_SPAWN_STACK;
        NATIVE_SPAWN_STACK = core::ptr::null_mut();
        let _ = crate::nagi_posix_munmap(stack, crate::threads::DEFAULT_STACK_SIZE);
    }
    NATIVE_SPAWN_RECORD = None;
    NATIVE_SPAWN_THREAD = None;
    Ok(result)
}

#[no_mangle]
pub extern "C" fn nagi_posix_fork() -> i64 {
    crate::errno::set_errno(crate::errno::ENOSYS);
    -1
}

#[no_mangle]
pub extern "C" fn nagi_posix_spawn() -> i64 {
    crate::errno::set_errno(crate::errno::ENOSYS);
    -1
}

#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "C" fn nagi_posix_spawn_entry(
    request: *const NagiSpawnRequest,
    thread: *mut u64,
) -> i32 {
    if request.is_null() || thread.is_null() {
        crate::errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    match native_spawn(&*request) {
        Ok(value) => {
            thread.write(value);
            0
        }
        Err(SpawnError::CapabilityEscalation) => {
            crate::errno::set_errno(crate::errno::EACCES);
            -1
        }
        Err(_) => {
            crate::errno::set_errno(crate::errno::EAGAIN);
            -1
        }
    }
}

#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "C" fn nagi_posix_spawn_wait(thread: u64, result: *mut u64) -> i32 {
    match native_wait(thread) {
        Ok(value) => {
            if !result.is_null() {
                result.write(value);
            }
            0
        }
        Err(_) => {
            crate::errno::set_errno(crate::errno::EAGAIN);
            -1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{attenuate_rights, posix_spawn, SpawnError};

    #[test]
    fn fork_is_not_a_hidden_kernel_requirement() {
        assert_eq!(posix_spawn(b"child", &[]), Err(SpawnError::Unsupported));
    }

    #[test]
    fn native_spawn_rights_only_attenuate() {
        assert_eq!(attenuate_rights(0b111, 0b101), Some(0b101));
        assert_eq!(attenuate_rights(0b101, 0b111), None);
    }
}
