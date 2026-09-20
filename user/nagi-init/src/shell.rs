use core::arch::asm;

use libnagi::storage::{DirectoryEntry, StorageError, SyscallBlockDevice, Vfs, BLOCK_SIZE};

type GuestVolume = Vfs<SyscallBlockDevice>;

const LINE_CAPACITY: usize = 128;
const MAX_LOG_OUTPUT: usize = libnagi::MAX_LOG_READ;

#[no_mangle]
static NAGI_SHELL_PROMPT: [u8; b"nsh> ".len()] = *b"nsh> ";
#[no_mangle]
static NAGI_SHELL_NEWLINE: [u8; b"\r\n".len()] = *b"\r\n";
#[no_mangle]
static NAGI_SHELL_ACCEPTANCE_PASS: [u8; b"Nagi M8 acceptance PASS\r\n".len()] =
    *b"Nagi M8 acceptance PASS\r\n";
#[no_mangle]
static NAGI_SHELL_ACCEPTANCE_FAIL: [u8; b"Nagi M8 acceptance FAIL\r\n".len()] =
    *b"Nagi M8 acceptance FAIL\r\n";
#[no_mangle]
static NAGI_SHELL_HELP: [u8;
    b"pwd ls cd cat echo touch write cp mv rm mkdir nagi ps|mem|log help exit\r\n".len()] =
    *b"pwd ls cd cat echo touch write cp mv rm mkdir nagi ps|mem|log help exit\r\n";
#[no_mangle]
static NAGI_SHELL_ROOT: [u8; b"/\r\n".len()] = *b"/\r\n";
#[no_mangle]
static NAGI_SHELL_COMMAND_ERROR: [u8; b"nsh: command failed\r\n".len()] =
    *b"nsh: command failed\r\n";
#[no_mangle]
static NAGI_SHELL_ARGUMENT_ERROR: [u8; b"nsh: invalid arguments\r\n".len()] =
    *b"nsh: invalid arguments\r\n";
#[no_mangle]
static NAGI_SHELL_UNKNOWN_COMMAND: [u8; b"nsh: unknown command\r\n".len()] =
    *b"nsh: unknown command\r\n";
#[no_mangle]
static NAGI_SHELL_START: [u8; b"Nagi M8 nsh START\r\n".len()] = *b"Nagi M8 nsh START\r\n";
#[no_mangle]
static NAGI_SHELL_BACKSPACE: [u8; b"\x08 \x08".len()] = *b"\x08 \x08";
#[no_mangle]
static NAGI_SHELL_WORD_PWD: [u8; b"pwd".len()] = *b"pwd";
#[no_mangle]
static NAGI_SHELL_WORD_LS: [u8; b"ls".len()] = *b"ls";
#[no_mangle]
static NAGI_SHELL_WORD_CD: [u8; b"cd".len()] = *b"cd";
#[no_mangle]
static NAGI_SHELL_WORD_CAT: [u8; b"cat".len()] = *b"cat";
#[no_mangle]
static NAGI_SHELL_WORD_ECHO: [u8; b"echo".len()] = *b"echo";
#[no_mangle]
static NAGI_SHELL_WORD_TOUCH: [u8; b"touch".len()] = *b"touch";
#[no_mangle]
static NAGI_SHELL_WORD_WRITE: [u8; b"write".len()] = *b"write";
#[no_mangle]
static NAGI_SHELL_WORD_CP: [u8; b"cp".len()] = *b"cp";
#[no_mangle]
static NAGI_SHELL_WORD_MV: [u8; b"mv".len()] = *b"mv";
#[no_mangle]
static NAGI_SHELL_WORD_RM: [u8; b"rm".len()] = *b"rm";
#[no_mangle]
static NAGI_SHELL_WORD_MKDIR: [u8; b"mkdir".len()] = *b"mkdir";
#[no_mangle]
static NAGI_SHELL_WORD_NAGI: [u8; b"nagi".len()] = *b"nagi";
#[no_mangle]
static NAGI_SHELL_WORD_PS: [u8; b"ps".len()] = *b"ps";
#[no_mangle]
static NAGI_SHELL_WORD_MEM: [u8; b"mem".len()] = *b"mem";
#[no_mangle]
static NAGI_SHELL_WORD_LOG: [u8; b"log".len()] = *b"log";
#[no_mangle]
static NAGI_SHELL_WORD_HELP: [u8; b"help".len()] = *b"help";
#[no_mangle]
static NAGI_SHELL_WORD_EXIT: [u8; b"exit".len()] = *b"exit";
#[no_mangle]
static NAGI_SHELL_ROOT_ARG: [u8; b"/".len()] = *b"/";
#[no_mangle]
static NAGI_SHELL_DOT_ARG: [u8; b".".len()] = *b".";
#[no_mangle]
static NAGI_SHELL_CURRENT_ARG: [u8; b"--current".len()] = *b"--current";
#[no_mangle]
static NAGI_SHELL_IMAGE_PREFIX: [u8; b" image_pages=".len()] = *b" image_pages=";
#[no_mangle]
static NAGI_SHELL_STACK_PREFIX: [u8; b" stack_pages=".len()] = *b" stack_pages=";
#[no_mangle]
static NAGI_SHELL_MEMORY_IMAGE_PREFIX: [u8; b"image_pages=".len()] = *b"image_pages=";
#[no_mangle]
static NAGI_SHELL_MEMORY_STACK_PREFIX: [u8; b"stack_pages=".len()] = *b"stack_pages=";
#[no_mangle]
static NAGI_SHELL_MEMORY_TLS_PREFIX: [u8; b"tls_pages=".len()] = *b"tls_pages=";
#[no_mangle]
static NAGI_SHELL_PID_PREFIX: [u8; b"pid=".len()] = *b"pid=";
#[no_mangle]
static NAGI_SHELL_NAME_PREFIX: [u8; b" name=".len()] = *b" name=";
#[no_mangle]
static NAGI_SHELL_STATE_SUFFIX: [u8; b" state=running\r\n".len()] = *b" state=running\r\n";
#[no_mangle]
static NAGI_SHELL_PWD_PASS: [u8; b"Nagi M8 pwd PASS\r\n".len()] = *b"Nagi M8 pwd PASS\r\n";
#[no_mangle]
static NAGI_SHELL_LS_PASS: [u8; b"Nagi M8 ls PASS\r\n".len()] = *b"Nagi M8 ls PASS\r\n";
#[no_mangle]
static NAGI_SHELL_CAT_PASS: [u8; b"Nagi M8 cat PASS\r\n".len()] = *b"Nagi M8 cat PASS\r\n";
#[no_mangle]
static NAGI_SHELL_PS_PASS: [u8; b"Nagi M8 ps PASS\r\n".len()] = *b"Nagi M8 ps PASS\r\n";
#[no_mangle]
static NAGI_SHELL_MEM_PASS: [u8; b"Nagi M8 mem PASS\r\n".len()] = *b"Nagi M8 mem PASS\r\n";
#[no_mangle]
static NAGI_SHELL_LOG_PASS: [u8; b"Nagi M8 log PASS\r\n".len()] = *b"Nagi M8 log PASS\r\n";

macro_rules! message {
    ($symbol:ident) => {{
        let message: *const u8;
        unsafe {
            asm!(
                "lea {message}, [rip + {symbol}]",
                message = out(reg) message,
                symbol = sym $symbol,
                options(nostack, preserves_flags, readonly),
            );
            core::slice::from_raw_parts(message, $symbol.len())
        }
    }};
}

#[derive(Default)]
struct AcceptanceState {
    pwd: bool,
    ls: bool,
    cat: bool,
    ps: bool,
    mem: bool,
    log: bool,
}

pub fn run(volume: &mut GuestVolume) -> ! {
    print(message!(NAGI_SHELL_START));
    print(message!(NAGI_SHELL_HELP));
    let mut line = [0_u8; LINE_CAPACITY];
    let mut state = AcceptanceState::default();
    loop {
        print(message!(NAGI_SHELL_PROMPT));
        let length = read_line(&mut line);
        if length == 0 {
            continue;
        }
        let command_line = unsafe { core::slice::from_raw_parts(line.as_ptr(), length) };
        if execute_command(volume, command_line, &mut state) {
            libnagi::exit(0);
        }
    }
}

fn execute_command(volume: &mut GuestVolume, line: &[u8], state: &mut AcceptanceState) -> bool {
    let (command, arguments) = split_first_word(line);
    if equals(command, message!(NAGI_SHELL_WORD_PWD)) {
        print(message!(NAGI_SHELL_ROOT));
        print(message!(NAGI_SHELL_PWD_PASS));
        state.pwd = true;
    } else if equals(command, message!(NAGI_SHELL_WORD_LS)) {
        if list(volume).is_ok() {
            print(message!(NAGI_SHELL_LS_PASS));
            state.ls = true;
        } else {
            print(message!(NAGI_SHELL_COMMAND_ERROR));
        }
    } else if equals(command, message!(NAGI_SHELL_WORD_CD)) {
        if arguments.is_empty()
            || equals(arguments, message!(NAGI_SHELL_ROOT_ARG))
            || equals(arguments, message!(NAGI_SHELL_DOT_ARG))
        {
            print(message!(NAGI_SHELL_ROOT));
        } else {
            print(message!(NAGI_SHELL_ARGUMENT_ERROR));
        }
    } else if equals(command, message!(NAGI_SHELL_WORD_CAT)) {
        if cat(volume, arguments).is_ok() {
            print(message!(NAGI_SHELL_CAT_PASS));
            state.cat = true;
        } else {
            print(message!(NAGI_SHELL_COMMAND_ERROR));
        }
    } else if equals(command, message!(NAGI_SHELL_WORD_ECHO)) {
        print(arguments);
        print(message!(NAGI_SHELL_NEWLINE));
    } else if equals(command, message!(NAGI_SHELL_WORD_TOUCH)) {
        if touch(volume, arguments).is_err() {
            print(message!(NAGI_SHELL_COMMAND_ERROR));
        }
    } else if equals(command, message!(NAGI_SHELL_WORD_WRITE)) {
        if write_file(volume, arguments).is_err() {
            print(message!(NAGI_SHELL_COMMAND_ERROR));
        }
    } else if equals(command, message!(NAGI_SHELL_WORD_CP)) {
        if copy_file(volume, arguments).is_err() {
            print(message!(NAGI_SHELL_COMMAND_ERROR));
        }
    } else if equals(command, message!(NAGI_SHELL_WORD_MV)) {
        if move_file(volume, arguments).is_err() {
            print(message!(NAGI_SHELL_COMMAND_ERROR));
        }
    } else if equals(command, message!(NAGI_SHELL_WORD_RM)) {
        if volume.remove(arguments).is_err() {
            print(message!(NAGI_SHELL_COMMAND_ERROR));
        }
    } else if equals(command, message!(NAGI_SHELL_WORD_MKDIR)) {
        if volume.mkdir(arguments).is_err() {
            print(message!(NAGI_SHELL_COMMAND_ERROR));
        }
    } else if equals(command, message!(NAGI_SHELL_WORD_NAGI)) {
        let (subcommand, subarguments) = split_first_word(arguments);
        if !subarguments.is_empty() && !equals(subarguments, message!(NAGI_SHELL_CURRENT_ARG)) {
            print(message!(NAGI_SHELL_ARGUMENT_ERROR));
        } else if equals(subcommand, message!(NAGI_SHELL_WORD_PS)) {
            if show_process() {
                print(message!(NAGI_SHELL_PS_PASS));
                state.ps = true;
            }
        } else if equals(subcommand, message!(NAGI_SHELL_WORD_MEM)) {
            if show_memory() {
                print(message!(NAGI_SHELL_MEM_PASS));
                state.mem = true;
            }
        } else if equals(subcommand, message!(NAGI_SHELL_WORD_LOG)) {
            if show_log() {
                print(message!(NAGI_SHELL_LOG_PASS));
                state.log = true;
            }
        } else {
            print(message!(NAGI_SHELL_ARGUMENT_ERROR));
        }
    } else if equals(command, message!(NAGI_SHELL_WORD_HELP)) {
        print(message!(NAGI_SHELL_HELP));
    } else if equals(command, message!(NAGI_SHELL_WORD_EXIT)) {
        if state.pwd && state.ls && state.cat && state.ps && state.mem && state.log {
            print(message!(NAGI_SHELL_ACCEPTANCE_PASS));
        } else {
            print(message!(NAGI_SHELL_ACCEPTANCE_FAIL));
        }
        return true;
    } else {
        print(message!(NAGI_SHELL_UNKNOWN_COMMAND));
    }
    false
}

fn list(volume: &mut GuestVolume) -> Result<(), StorageError> {
    let mut entries = [DirectoryEntry::empty(); libnagi::storage::MAX_DIRECTORY_ENTRIES];
    let count = volume.list_root(&mut entries)?;
    let mut index = 0;
    while index < count {
        let entry = unsafe { entries.as_ptr().add(index).read() };
        print(entry.name());
        print(message!(NAGI_SHELL_NEWLINE));
        index += 1;
    }
    Ok(())
}

fn cat(volume: &mut GuestVolume, name: &[u8]) -> Result<(), StorageError> {
    let handle = volume.open(name)?;
    let mut buffer = [0_u8; BLOCK_SIZE];
    let length = volume.read(handle, &mut buffer)?;
    let content = unsafe { core::slice::from_raw_parts(buffer.as_ptr(), length) };
    print(content);
    let last_byte = if length == 0 {
        0
    } else {
        unsafe { core::ptr::read_volatile(buffer.as_ptr().add(length - 1)) }
    };
    if length == 0 || last_byte != b'\n' {
        print(message!(NAGI_SHELL_NEWLINE));
    }
    Ok(())
}

fn touch(volume: &mut GuestVolume, name: &[u8]) -> Result<(), StorageError> {
    volume.create(name).map(|_| ())
}

fn write_file(volume: &mut GuestVolume, arguments: &[u8]) -> Result<(), StorageError> {
    let (name, data) = split_first_word(arguments);
    if name.is_empty() || data.len() > BLOCK_SIZE {
        return Err(StorageError::InvalidName);
    }
    let handle = match volume.create(name) {
        Ok(handle) => handle,
        Err(StorageError::AlreadyExists) => volume.open(name)?,
        Err(error) => return Err(error),
    };
    volume.write(handle, data)
}

fn copy_file(volume: &mut GuestVolume, arguments: &[u8]) -> Result<(), StorageError> {
    let (source_name, destination_name) = split_first_word(arguments);
    let (destination_name, trailing) = split_first_word(destination_name);
    if source_name.is_empty() || destination_name.is_empty() || !trailing.is_empty() {
        return Err(StorageError::InvalidName);
    }
    let source = volume.open(source_name)?;
    let mut buffer = [0_u8; BLOCK_SIZE];
    let length = volume.read(source, &mut buffer)?;
    let destination = volume.create(destination_name)?;
    let source_bytes = unsafe { core::slice::from_raw_parts(buffer.as_ptr(), length) };
    volume.write(destination, source_bytes)
}

fn move_file(volume: &mut GuestVolume, arguments: &[u8]) -> Result<(), StorageError> {
    copy_file(volume, arguments)?;
    let (source_name, destination_name) = split_first_word(arguments);
    let (_, trailing) = split_first_word(destination_name);
    if source_name.is_empty() || destination_name.is_empty() || !trailing.is_empty() {
        return Err(StorageError::InvalidName);
    }
    volume.remove(source_name)
}

fn show_process() -> bool {
    let mut info = libnagi::ProcessInfo {
        pid: 0,
        parent_pid: 0,
        state: 0,
        flags: 0,
        image_pages: 0,
        stack_pages: 0,
        name: [0; libnagi::MAX_PROCESS_NAME],
    };
    if !libnagi::process_info(&mut info) {
        print(message!(NAGI_SHELL_COMMAND_ERROR));
        return false;
    }
    print(message!(NAGI_SHELL_PID_PREFIX));
    print_u64_value(info.pid);
    print(message!(NAGI_SHELL_NAME_PREFIX));
    let mut name_length = 0;
    while name_length < info.name.len()
        && unsafe { core::ptr::read_volatile(info.name.as_ptr().add(name_length)) } != 0
    {
        name_length += 1;
    }
    let name = unsafe { core::slice::from_raw_parts(info.name.as_ptr(), name_length) };
    print(name);
    print(message!(NAGI_SHELL_STATE_SUFFIX));
    print_u64(
        message!(NAGI_SHELL_IMAGE_PREFIX),
        u64::from(info.image_pages),
    );
    print_u64(
        message!(NAGI_SHELL_STACK_PREFIX),
        u64::from(info.stack_pages),
    );
    true
}

fn show_memory() -> bool {
    let mut info = libnagi::MemoryInfo {
        image_pages: 0,
        stack_pages: 0,
        tls_pages: 0,
        image_base: 0,
        image_limit: 0,
        stack_base: 0,
        stack_limit: 0,
    };
    if !libnagi::memory_info(&mut info) {
        print(message!(NAGI_SHELL_COMMAND_ERROR));
        return false;
    }
    print_u64(message!(NAGI_SHELL_MEMORY_IMAGE_PREFIX), info.image_pages);
    print_u64(message!(NAGI_SHELL_MEMORY_STACK_PREFIX), info.stack_pages);
    print_u64(message!(NAGI_SHELL_MEMORY_TLS_PREFIX), info.tls_pages);
    true
}

fn show_log() -> bool {
    let mut buffer = [0_u8; MAX_LOG_OUTPUT];
    let length = libnagi::log_read(&mut buffer);
    if length == 0 {
        print(message!(NAGI_SHELL_COMMAND_ERROR));
        return false;
    }
    let content = unsafe { core::slice::from_raw_parts(buffer.as_ptr(), length) };
    print(content);
    true
}

fn read_line(line: &mut [u8; LINE_CAPACITY]) -> usize {
    let mut length = 0;
    loop {
        let mut byte = [0_u8; libnagi::MAX_CONSOLE_READ];
        if !libnagi::console_read(&mut byte) {
            core::hint::spin_loop();
            continue;
        }
        match byte[0] {
            b'\r' | b'\n' => {
                print(message!(NAGI_SHELL_NEWLINE));
                return length;
            }
            8 | 127 if length > 0 => {
                length -= 1;
                print(message!(NAGI_SHELL_BACKSPACE));
            }
            32..=126 if length + 1 < line.len() => {
                unsafe { core::ptr::write_volatile(line.as_mut_ptr().add(length), byte[0]) };
                length += 1;
                print(&byte);
            }
            _ => {}
        }
    }
}

fn split_first_word(input: &[u8]) -> (&[u8], &[u8]) {
    let mut start = 0;
    while start < input.len() && input[start] == b' ' {
        start += 1;
    }
    let mut end = start;
    while end < input.len() && input[end] != b' ' {
        end += 1;
    }
    let mut remainder = end;
    while remainder < input.len() && input[remainder] == b' ' {
        remainder += 1;
    }
    let word = unsafe { core::slice::from_raw_parts(input.as_ptr().add(start), end - start) };
    let rest = unsafe {
        core::slice::from_raw_parts(input.as_ptr().add(remainder), input.len() - remainder)
    };
    (word, rest)
}

fn equals(left: &[u8], right: &[u8]) -> bool {
    left == right
}

fn print(bytes: &[u8]) {
    let mut offset = 0;
    while offset < bytes.len() {
        let count = (bytes.len() - offset).min(libnagi::MAX_CONSOLE_WRITE);
        let chunk = unsafe { core::slice::from_raw_parts(bytes.as_ptr().add(offset), count) };
        libnagi::console_write(chunk);
        offset += count;
    }
}

fn print_u64(prefix: &[u8], value: u64) {
    print(prefix);
    print_u64_value(value);
    print(message!(NAGI_SHELL_NEWLINE));
}

fn print_u64_value(value: u64) {
    let mut digits = [0_u8; 20];
    let mut index = digits.len();
    let mut value = value;
    if value == 0 {
        index -= 1;
        digits[index] = b'0';
    } else {
        while value != 0 {
            index -= 1;
            digits[index] = b'0' + (value % 10) as u8;
            value /= 10;
        }
    }
    let number =
        unsafe { core::slice::from_raw_parts(digits.as_ptr().add(index), digits.len() - index) };
    print(number);
}
