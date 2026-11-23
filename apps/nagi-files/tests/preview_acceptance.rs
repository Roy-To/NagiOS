use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nagi_files::{
    Actor, CapabilityGrant, CapabilityRight, CapabilitySet, FilesCanvas, FilesCanvasRect,
    FilesComponent, FilesErrorKind, FilesService, Location, SandboxProvider,
};
use nagi_ui::{ColorRole, TypeRole};

static NEXT_PREVIEW_SANDBOX: AtomicU64 = AtomicU64::new(1);

struct PreviewSandbox(PathBuf);

impl PreviewSandbox {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "nagi-files-preview-acceptance-{}-{nonce}-{}",
            std::process::id(),
            NEXT_PREVIEW_SANDBOX.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create a private acceptance sandbox");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PreviewSandbox {
    fn drop(&mut self) {
        let allocated_bytes = allocated_bytes(&self.0).unwrap_or_default();
        fs::remove_dir_all(&self.0).expect("remove the task-owned acceptance sandbox");
        println!(
            "NAGI_FILES_PREVIEW_ACCEPTANCE_SCRATCH_CLEANED path={} allocated_bytes={allocated_bytes}",
            self.0.display()
        );
    }
}

#[derive(Default)]
struct FilesCanvasCapture {
    texts: Vec<String>,
}

fn run_host_preview(root: &Path, permissions: &str, commands: &[u8]) -> std::process::Output {
    let mut preview = Command::new(env!("CARGO_BIN_EXE_nagi-files-preview"))
        .arg(root)
        .args(["--allow", permissions])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the Files host preview against the task-owned sandbox");
    preview
        .stdin
        .take()
        .expect("preview stdin is piped")
        .write_all(commands)
        .expect("send commands to the Files host preview");
    preview
        .wait_with_output()
        .expect("wait for the Files host preview to exit")
}

fn sandbox_component(
    sandbox: &PreviewSandbox,
    grants: impl IntoIterator<Item = CapabilityGrant>,
) -> FilesComponent<SandboxProvider, CapabilitySet> {
    let provider = SandboxProvider::new(sandbox.path()).expect("open the task-owned sandbox");
    FilesComponent::new(
        FilesService::new(provider, CapabilitySet::from_grants(grants)),
        Location::root(),
    )
}

fn assert_opened_resource(
    operation: &nagi_files::OperationResult,
    resource_id: nagi_files::ResourceId,
) {
    assert_eq!(operation.action_id, "files.open");
    assert_eq!(operation.affected, vec![resource_id]);
}

fn assert_file_bytes(path: impl AsRef<Path>, expected: &[u8]) {
    let path = path.as_ref();
    let actual = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert_eq!(actual, expected, "persisted bytes at {}", path.display());
}

impl FilesCanvas for FilesCanvasCapture {
    fn fill_rect(&mut self, _bounds: FilesCanvasRect, _color: ColorRole) {}

    fn frame_rect(&mut self, _bounds: FilesCanvasRect, _color: ColorRole) {}

    fn text(
        &mut self,
        _bounds: FilesCanvasRect,
        value: &str,
        _typography: TypeRole,
        _color: ColorRole,
    ) {
        self.texts.push(value.to_owned());
    }
}

#[test]
fn host_preview_saves_and_reopens_the_focused_file_in_one_session() {
    const FILE_NAME: &str = "draft.txt";
    const ORIGINAL: &str = "original body\n";
    const SAVED: &str = "updated body with spaces";

    let sandbox = PreviewSandbox::new();
    fs::write(sandbox.path().join(FILE_NAME), ORIGINAL.as_bytes())
        .expect("seed the selected file in the task-owned sandbox");
    let output = run_host_preview(
        sandbox.path(),
        "read,enumerate,write",
        b"select 1\nopen\nsave updated body with spaces\nrefresh\nopen\nquit\n",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "preview failed: status={}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    assert_eq!(stdout.matches("files.write applied").count(), 1, "{stdout}");
    let original_position = stdout.find(ORIGINAL).unwrap();
    let write_result_position = stdout.find("files.write applied").unwrap();
    let reopened_position = stdout.find(SAVED).unwrap();
    assert!(
        original_position < write_result_position && write_result_position < reopened_position,
        "the write result must appear once between the original and refreshed read: {stdout}"
    );
    assert_file_bytes(sandbox.path().join(FILE_NAME), SAVED.as_bytes());
}

#[test]
fn host_component_reopens_latest_contents_after_parent_navigation_with_same_resource_id() {
    const FILE_NAME: &str = "report.txt";
    const ORIGINAL: &[u8] = b"contents before navigating away\n";
    const UPDATED: &[u8] = b"contents changed while another folder was open\n";

    let sandbox = PreviewSandbox::new();
    fs::create_dir(sandbox.path().join("browse")).expect("create the child folder");
    fs::write(sandbox.path().join(FILE_NAME), ORIGINAL).expect("seed the parent file");
    let file_location = Location::parse(FILE_NAME).unwrap();
    let browse_location = Location::parse("browse").unwrap();
    let mut component = sandbox_component(
        &sandbox,
        [
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(browse_location.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(file_location, CapabilityRight::Read),
        ],
    );
    component.refresh().unwrap();
    let original = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == FILE_NAME)
        .unwrap()
        .clone();
    component.select(original.id, false).unwrap();
    let (opened_before, _) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(opened_before, ORIGINAL);

    component.navigate(browse_location).unwrap();
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    fs::write(sandbox.path().join(FILE_NAME), UPDATED)
        .expect("update the parent file while its listing is not visible");

    assert!(component.navigate_parent().unwrap());
    assert_eq!(component.state().location, Location::root());
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    let returned = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == FILE_NAME)
        .unwrap();
    assert_eq!(returned.id, original.id);
    assert_eq!(returned.size_bytes, Some(UPDATED.len() as u64));

    component.select(returned.id, false).unwrap();
    let (opened_after, operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_opened_resource(&operation, original.id);
    assert_eq!(opened_after, UPDATED);
}

#[test]
fn host_component_save_after_dismissing_error_shows_current_selection() {
    const FILE_NAME: &str = "draft.txt";
    const ORIGINAL: &[u8] = b"original body\n";
    const SAVED: &[u8] = b"saved body with updated size\n";

    let sandbox = PreviewSandbox::new();
    fs::write(sandbox.path().join(FILE_NAME), ORIGINAL).expect("seed the selected file");
    let file_location = Location::parse(FILE_NAME).unwrap();
    let mut component = sandbox_component(
        &sandbox,
        [
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(file_location, CapabilityRight::Write),
        ],
    );
    component.refresh().unwrap();
    let selected = component.state().entries[0].clone();
    component.select(selected.id, false).unwrap();

    assert_eq!(
        component.open_selected(Actor::User).unwrap_err().kind,
        FilesErrorKind::PermissionDenied
    );
    let permission_message = nagi_files::localization::error_message(
        nagi_files::localization::Locale::JaJp,
        &nagi_files::FilesError::new(FilesErrorKind::PermissionDenied),
    );
    let mut canvas = FilesCanvasCapture::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 120),
        "This folder is empty.",
        nagi_files::localization::Locale::JaJp,
    );
    assert!(canvas.texts.contains(&permission_message));

    component.select(selected.id, false).unwrap();
    let (saved, operation) = component.save_selected(SAVED, Actor::User).unwrap();
    assert_eq!(operation.action_id, "files.write");
    assert_eq!(saved.id, selected.id);
    assert_eq!(saved.size_bytes, Some(SAVED.len() as u64));
    assert_file_bytes(sandbox.path().join(FILE_NAME), SAVED);
    let inspector = component.state().layout().inspector.unwrap();
    assert_eq!(inspector.id, selected.id);
    assert_eq!(inspector.name.as_str(), FILE_NAME);
    assert_eq!(inspector.size_bytes, Some(SAVED.len() as u64));

    let mut canvas = FilesCanvasCapture::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 120),
        "This folder is empty.",
        nagi_files::localization::Locale::JaJp,
    );
    assert!(!canvas.texts.contains(&permission_message));
    assert!(canvas.texts.iter().any(|text| text == FILE_NAME));
}

#[test]
fn host_component_file_info_matches_long_japanese_open_bytes() {
    const FILE_NAME: &str = "長文_日本語.txt";
    let contents = "これはUTF-8の長い日本語本文です。FileInfoのサイズは文字数でなく保存バイト数と一致します。\n"
        .repeat(96)
        .into_bytes();

    let sandbox = PreviewSandbox::new();
    fs::write(sandbox.path().join(FILE_NAME), &contents).expect("seed the Japanese text file");
    let file_location = Location::parse(FILE_NAME).unwrap();
    let mut component = sandbox_component(
        &sandbox,
        [
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(file_location, CapabilityRight::Read),
        ],
    );
    component.refresh().unwrap();
    let entry = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == FILE_NAME)
        .unwrap()
        .clone();
    component.select(entry.id, false).unwrap();

    let inspector = component.state().layout().inspector.unwrap();
    assert_eq!(inspector.id, entry.id);
    assert_eq!(inspector.size_bytes, Some(contents.len() as u64));
    let mut canvas = FilesCanvasCapture::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        nagi_files::localization::Locale::EnUs,
    );
    assert!(canvas.texts.iter().any(|text| text == FILE_NAME));
    assert!(canvas
        .texts
        .iter()
        .any(|text| text == &format!("{} B", contents.len())));

    let (opened, operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_opened_resource(&operation, entry.id);
    assert_eq!(opened, contents);
}

fn allocated_bytes(path: &Path) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::symlink_metadata(path)?;
    let own = metadata.blocks().saturating_mul(512);
    if metadata.is_dir() {
        fs::read_dir(path)?.try_fold(own, |total, entry| {
            allocated_bytes(&entry?.path()).map(|size| total.saturating_add(size))
        })
    } else {
        Ok(own)
    }
}

#[test]
fn host_preview_accepts_normal_folder_create_and_rename_in_one_session() {
    const CREATED_NAME: &str = "日本語フォルダ";
    const RENAMED_NAME: &str = "日本語フォルダ_改名後";

    let sandbox = PreviewSandbox::new();
    let commands = format!("mkdir {CREATED_NAME}\nrename 1 {RENAMED_NAME}\nquit\n");
    let output = run_host_preview(
        sandbox.path(),
        "enumerate,create,rename",
        commands.as_bytes(),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "preview failed: status={}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    assert!(
        stdout.contains("host preview; Nagi target integration: not run"),
        "the preview must label the unintegrated host boundary: {stdout}"
    );
    assert!(stdout.contains("files.create_folder applied"), "{stdout}");
    assert!(stdout.contains("files.rename applied"), "{stdout}");
    assert!(
        stdout.contains(&format!("[D] {RENAMED_NAME}")),
        "the refreshed listing must show the renamed folder: {stdout}"
    );
    assert!(sandbox.path().join(RENAMED_NAME).is_dir());
    assert!(!sandbox.path().join(CREATED_NAME).exists());
}

#[test]
fn host_preview_reopens_other_and_tagged_files_without_mixing_inspector_or_contents() {
    const ORIGINAL_NAME: &str = "a-original.txt";
    const OTHER_NAME: &str = "b-other.txt";
    const ORIGINAL_CONTENTS: &str = "original file body\n";
    const SAVED_ORIGINAL_CONTENTS: &str = "saved original file body";
    const OTHER_CONTENTS: &str = "other file body\n";

    let sandbox = PreviewSandbox::new();
    fs::write(
        sandbox.path().join(ORIGINAL_NAME),
        ORIGINAL_CONTENTS.as_bytes(),
    )
    .expect("seed the original file in the task-owned sandbox");
    fs::write(sandbox.path().join(OTHER_NAME), OTHER_CONTENTS.as_bytes())
        .expect("seed the other file in the task-owned sandbox");
    let commands = format!(
        "select 1\ntags 1 archive,日本語\nsave {SAVED_ORIGINAL_CONTENTS}\ntoggle-inspector\nselect 2\ntoggle-inspector\nopen\nselect 1\nopen\nquit\n"
    );
    let output = run_host_preview(
        sandbox.path(),
        "read,enumerate,metadata,write",
        commands.as_bytes(),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "preview failed: status={}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    assert!(stdout.contains("files.set_tags applied"), "{stdout}");
    assert_eq!(stdout.matches("files.write applied").count(), 1, "{stdout}");
    assert!(stdout.contains(ORIGINAL_NAME), "{stdout}");
    assert!(stdout.contains(OTHER_NAME), "{stdout}");
    assert!(
        stdout.find(OTHER_CONTENTS).unwrap() < stdout.find(SAVED_ORIGINAL_CONTENTS).unwrap(),
        "the reopened other-file body must precede the saved original body: {stdout}"
    );
    assert_file_bytes(
        sandbox.path().join(ORIGINAL_NAME),
        SAVED_ORIGINAL_CONTENTS.as_bytes(),
    );
    assert_file_bytes(sandbox.path().join(OTHER_NAME), OTHER_CONTENTS.as_bytes());

    let mut current_snapshot = Vec::new();
    let mut inspector_snapshots = Vec::new();
    for line in stdout.lines() {
        if line.contains("| Inspector (Metadata)") {
            if !current_snapshot.is_empty() {
                inspector_snapshots.push(std::mem::take(&mut current_snapshot));
            }
        }
        if line.contains("| Inspector (Metadata)") || !current_snapshot.is_empty() {
            current_snapshot.push(line);
        }
    }
    if !current_snapshot.is_empty() {
        inspector_snapshots.push(current_snapshot);
    }

    for name in [ORIGINAL_NAME, OTHER_NAME] {
        assert!(
            inspector_snapshots.iter().any(|snapshot| {
                let selected_in_resources = snapshot.iter().any(|line| {
                    line.split('|')
                        .nth(1)
                        .is_some_and(|resources| resources.contains(&format!("*[F] {name}")))
                });
                let shown_in_inspector = snapshot.iter().any(|line| {
                    line.split('|')
                        .nth(2)
                        .is_some_and(|inspector| inspector.trim() == name)
                });
                selected_in_resources && shown_in_inspector
            }),
            "the selected row and Inspector must identify {name} in the same view: {stdout}"
        );
    }

    let provider = nagi_files::SandboxProvider::new(sandbox.path())
        .expect("reopen the task-owned sandbox after the preview exits");
    let entries = nagi_files::FilesystemProvider::list(&provider, &nagi_files::Location::root())
        .expect("read the normal preview's final listing");
    let original = entries
        .iter()
        .find(|entry| entry.name.as_str() == ORIGINAL_NAME)
        .expect("the tagged original remains in the listing");
    let other = entries
        .iter()
        .find(|entry| entry.name.as_str() == OTHER_NAME)
        .expect("the other file remains in the listing");
    assert_eq!(original.tags, vec!["archive", "日本語"]);
    assert!(other.tags.is_empty());
}

#[test]
fn host_preview_clears_moved_selection_after_nested_parent_return() {
    const PARENT: &str = "parent";
    const CHILD: &str = "child";
    const ARCHIVE: &str = "archive";
    const MOVED_NAME: &str = "a-moved.txt";
    const SIBLING_NAME: &str = "b-sibling.txt";

    let sandbox = PreviewSandbox::new();
    fs::create_dir_all(sandbox.path().join(PARENT).join(CHILD).join(ARCHIVE))
        .expect("prepare the nested source and destination folders");
    fs::write(
        sandbox.path().join(PARENT).join(CHILD).join("a-before.txt"),
        b"move the previously selected row\n",
    )
    .expect("seed the movable file in the nested source folder");
    fs::write(
        sandbox.path().join(PARENT).join(CHILD).join(SIBLING_NAME),
        b"keep the other source row\n",
    )
    .expect("seed the source sibling that must remain");
    let output = run_host_preview(
        sandbox.path(),
        "read,enumerate,move,create",
        b"cd parent\ncd child\nnext\nnext\nprevious\nnext\nmove 2 archive a-moved.txt\ncd archive\nup\nquit\n",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "preview failed: status={}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    assert!(stdout.contains("files.move applied"), "{stdout}");
    let final_parent_view = stdout
        .rsplit_once("Home / parent / child")
        .map(|(_, view)| view)
        .expect("the preview returns to the nested source parent");
    assert!(
        final_parent_view.contains("[D] archive"),
        "{final_parent_view}"
    );
    assert!(
        final_parent_view.contains(&format!("[F] {SIBLING_NAME}")),
        "the remaining source sibling stays listed: {final_parent_view}"
    );
    assert!(
        !final_parent_view.contains(MOVED_NAME),
        "the moved file must be absent from its former parent: {final_parent_view}"
    );
    assert!(
        !final_parent_view.contains("*["),
        "the parent view has no stale selection marker: {final_parent_view}"
    );
    assert!(
        final_parent_view.contains("No selection"),
        "the Inspector is empty after returning to the parent: {final_parent_view}"
    );
    assert!(sandbox
        .path()
        .join(PARENT)
        .join(CHILD)
        .join(ARCHIVE)
        .join(MOVED_NAME)
        .is_file());
    assert!(sandbox
        .path()
        .join(PARENT)
        .join(CHILD)
        .join(SIBLING_NAME)
        .is_file());
}
