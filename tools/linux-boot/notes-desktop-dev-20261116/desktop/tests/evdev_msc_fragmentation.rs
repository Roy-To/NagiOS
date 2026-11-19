use std::io::{self, Read};
#[cfg(target_os = "linux")]
use std::sync::Arc;

use libnagi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_TAB};
use linux_evdev_decoder::{LinuxEvdevTarget, EV_KEY, EV_MSC, EV_SYN, MSC_SCAN, SYN_REPORT};
use linux_evdev_stream::{EvdevStream, StreamItem};
#[cfg(target_os = "linux")]
use nagi_desktop_notes_integration_candidate::desktop::{
    new_linux_development_volume, Desktop, UserDataVolume,
};
#[cfg(target_os = "linux")]
use nagi_localization::Locale;
#[cfg(target_os = "linux")]
use notes_event_bridge_candidate::NotesEventBridge;
#[cfg(target_os = "linux")]
use notes_files_codec_candidate::{
    InMemoryNoteStore, NoteDocument, NoteStore, ObjectId, Timestamp,
};

/// Forces every 24-byte x86_64 evdev record through all possible interior split
/// points while leaving the production EvdevStream and decoder in control.
struct OneByteReader {
    bytes: Vec<u8>,
    offset: usize,
}

impl Read for OneByteReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.offset == self.bytes.len() {
            return Ok(0);
        }
        output[0] = self.bytes[self.offset];
        self.offset += 1;
        Ok(1)
    }
}

fn event(event_type: u16, code: u16, value: i32) -> InputEvent {
    InputEvent {
        event_type,
        code,
        value,
    }
}

fn x86_64_record(event: InputEvent) -> [u8; 24] {
    let mut record = [0; 24];
    record[16..18].copy_from_slice(&event.event_type.to_le_bytes());
    record[18..20].copy_from_slice(&event.code.to_le_bytes());
    record[20..24].copy_from_slice(&event.value.to_le_bytes());
    record
}

#[cfg(target_os = "linux")]
fn development_volume() -> (UserDataVolume, std::path::PathBuf) {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".evdev-regression-vfs.img");
    let _ = std::fs::remove_file(&path);
    // LinuxBlockDevice::open opens an existing regular file and derives its
    // capacity from the file length. Match the production consumer fixture:
    // create a task-local 8 MiB blank image before invoking the actual API.
    let image = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create the task-local development VFS image");
    image
        .set_len(8 * 1024 * 1024)
        .expect("size the task-local development VFS image");
    image
        .sync_all()
        .expect("flush the task-local development VFS image");
    drop(image);
    let (volume, formatted) = new_linux_development_volume(&path, true)
        .expect("format only the task-local development VFS image");
    assert!(formatted, "the task-local image starts empty");
    (volume, path)
}

#[test]
fn fragmented_msc_and_sync_records_do_not_drop_normal_ui_keys() {
    #[cfg(target_os = "linux")]
    let mut desktop = {
        let note_id = ObjectId(0x4e41_4749_0000_0013);
        let store = Arc::new(InMemoryNoteStore::new());
        store
            .commit(&NoteDocument::new(note_id, "EV_MSC input", Timestamp(1)), 0)
            .expect("seed the real Notes store API");

        let mut desktop = Desktop::new(Locale::EnUs);
        desktop
            .attach_notes(NotesEventBridge::new(store), note_id)
            .expect("attach the actual Notes bridge to the public Desktop API");
        desktop
    };
    #[cfg(target_os = "linux")]
    let (mut volume, cleanup_path) = development_volume();

    let input = [
        event(EV_KEY, INPUT_KEY_TAB, 1),
        event(EV_KEY, INPUT_KEY_TAB, 1),
        event(EV_KEY, INPUT_KEY_TAB, 1),
        event(EV_KEY, INPUT_KEY_ENTER, 1),
        event(EV_KEY, INPUT_KEY_ENTER, 0),
        event(EV_KEY, 30, 1), // Linux KEY_A, before the MSC run.
        event(EV_KEY, 30, 0),
        event(EV_MSC, MSC_SCAN, 0x70004),
        event(EV_MSC, MSC_SCAN, 0x70016), // Consecutive scan metadata records.
        event(EV_SYN, SYN_REPORT, 0),
        event(EV_KEY, 31, 1), // Linux KEY_S, after both MSC records.
        event(EV_KEY, 31, 0),
        event(EV_SYN, SYN_REPORT, 0),
        event(EV_MSC, MSC_SCAN, 0x70005),
        event(EV_KEY, 48, 1), // Linux KEY_B, after another MSC record.
        event(EV_KEY, 48, 0),
        event(EV_SYN, SYN_REPORT, 0),
    ];
    let bytes = input
        .iter()
        .flat_map(|event| x86_64_record(*event))
        .collect();
    let reader = OneByteReader { bytes, offset: 0 };
    let mut stream = EvdevStream::new(reader, LinuxEvdevTarget::X86_64);
    let mut decoded = Vec::with_capacity(input.len());

    loop {
        match stream
            .next_item()
            .expect("decode each complete evdev record")
        {
            StreamItem::Event(event) => {
                decoded.push(event);
                // This is the normal UI consumer branch in desktop/src/main.rs:
                // non-key input is ignored, and real key ABI events go to Desktop.
                if event.event_type == INPUT_EVENT_KEY {
                    #[cfg(target_os = "linux")]
                    desktop.handle_event(event, &mut volume);
                }
            }
            StreamItem::Eof => break,
            other => panic!("unexpected stream outcome: {other:?}"),
        }
    }

    assert_eq!(
        decoded, input,
        "keys, MSC metadata, and SYN records stay ordered"
    );
    #[cfg(target_os = "linux")]
    {
        assert!(desktop.linux_development_notes_window_active());
        assert_eq!(
            desktop
                .current_notes_document()
                .expect("Notes window retains the current document")
                .search_text(),
            "asb",
            "real key presses before, after, and following MSC/SYN records reach Notes"
        );
        drop(volume);
        std::fs::remove_file(cleanup_path).expect("remove the task-local VFS image");
        println!(
            "NAGI_EVDEV_MSC_FRAGMENTATION_PASS body=asb decoded_records={} vfs_removed=1",
            decoded.len()
        );
    }
}
