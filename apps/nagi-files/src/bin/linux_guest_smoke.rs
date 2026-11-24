#[cfg(not(target_os = "linux"))]
compile_error!("nagi-files-linux-guest-smoke must be built for a Linux target");

use std::path::{Path, PathBuf};
use std::time::Duration;

use nagi_files::localization::Locale;
use nagi_files::{
    Actor, CancellationToken, CapabilityGrant, CapabilityRight, CapabilitySet, EntryKind, FileName,
    FilesCanvas, FilesCanvasRect, FilesComponent, FilesErrorKind, FilesService, Location,
    OperationOutcome, Reversibility, SandboxProvider,
};
use nagi_ui::{ColorRole, TypeRole};

const DEFAULT_SANDBOX: &str = "/tmp/nagi-files-linux-smoke";
const DEFAULT_FOLDER: &str = "LinuxFilesSmoke";
const FIXTURE_NAME: &str = "LinuxFilesOpenSmoke.txt";
const COPIED_NAME: &str = "LinuxFilesCopiedSmoke.txt";
const MOVED_NAME: &str = "LinuxFilesMovedSmoke.txt";
const FIXTURE_CONTENTS: &str = "Nagi Files Linux open fixture contents\n";
const SAVED_CONTENTS: &str = "Nagi Files Linux saved contents\n";

#[derive(Default)]
struct GuestCanvas {
    texts: Vec<String>,
    selected_rows: usize,
}

impl FilesCanvas for GuestCanvas {
    fn fill_rect(&mut self, _bounds: FilesCanvasRect, color: ColorRole) {
        if color == ColorRole::Selection {
            self.selected_rows += 1;
        }
    }

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

fn main() {
    if let Err(error) = run() {
        eprintln!("NAGI_FILES_LINUX_GUEST_FAIL {error}");
    }

    // This consumer is the initramfs PID 1 in the bounded QEMU smoke run.
    // Keep the guest alive until the existing QEMU child runner sends QUIT.
    loop {
        std::thread::sleep(Duration::from_secs(300));
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let sandbox = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SANDBOX));
    let folder = match args.next() {
        Some(value) => value
            .into_string()
            .map_err(|_| "folder name must be valid UTF-8".to_owned())?,
        None => DEFAULT_FOLDER.to_owned(),
    };
    if args.next().is_some() {
        return Err(
            "usage: nagi-files-linux-guest-smoke [sandbox-under-/tmp] [folder-name]".into(),
        );
    }
    if sandbox.parent() != Some(Path::new("/tmp")) {
        return Err("sandbox must be a new direct child of /tmp".into());
    }

    std::fs::create_dir_all("/tmp").map_err(|error| format!("create /tmp: {error}"))?;
    std::fs::create_dir(&sandbox)
        .map_err(|error| format!("fresh sandbox is not available: {error}"))?;
    std::fs::write(sandbox.join(FIXTURE_NAME), FIXTURE_CONTENTS.as_bytes())
        .map_err(|error| format!("seed Files open fixture: {error}"))?;

    let initial_count = {
        let mut component = open_component(&sandbox, &folder)?;
        component
            .refresh()
            .map_err(|error| format!("initial Files listing: {error:?}"))?;
        let entries = &component.state().entries;
        if entries.len() != 1
            || entries[0].name.as_str() != FIXTURE_NAME
            || entries[0].kind != EntryKind::File
        {
            return Err("Files component listing did not show the open fixture".into());
        }
        entries.len()
    };

    let created_folder = {
        let mut component = open_component(&sandbox, &folder)?;
        component
            .refresh()
            .map_err(|error| format!("pre-create Files listing: {error:?}"))?;
        let name = FileName::parse(&folder).map_err(|error| format!("folder name: {error:?}"))?;
        let (entry, operation) = component
            .create_folder(&name, Actor::User)
            .map_err(|error| format!("create folder through Files component: {error:?}"))?;
        if entry.kind != EntryKind::Folder
            || operation.action_id != "files.create_folder"
            || operation.outcome != OperationOutcome::Applied
        {
            return Err("Files component create-folder result did not match".into());
        }
        entry
    };

    let (
        reloaded_count,
        entered_breadcrumbs,
        breadcrumb_path,
        inspector_name,
        opened_contents,
        copied_content_verified,
        moved_content_verified,
        restored_content_verified,
        trash_items_after_restore,
    ) = {
        // Reopen the provider to verify the Files API reads the committed
        // directory entry and fixture contents from the guest filesystem.
        let mut component = open_component(&sandbox, &folder)?;
        component
            .refresh()
            .map_err(|error| format!("reopened Files listing: {error:?}"))?;
        let reloaded_count = component.state().entries.len();
        let folder_location = created_folder.child_location();
        let visible_folder = component
            .state()
            .entries
            .iter()
            .find(|entry| entry.id == created_folder.id && entry.kind == EntryKind::Folder)
            .ok_or("created folder was absent after provider reload")?;
        if visible_folder.child_location() != folder_location {
            return Err("reloaded folder identity did not match the created folder".into());
        }
        component
            .navigate(folder_location)
            .map_err(|error| format!("open folder through Files component: {error:?}"))?;
        if component.state().location != created_folder.child_location() {
            return Err("component did not navigate into the created folder".into());
        }
        if component
            .state()
            .breadcrumbs
            .last()
            .map(|item| item.label.as_str())
            != Some(folder.as_str())
        {
            return Err("folder breadcrumb was not updated after navigation".into());
        }
        let entered_breadcrumbs = component
            .state()
            .breadcrumbs
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>()
            .join("/");
        if !component.state().entries.is_empty() {
            return Err("new folder listing was not empty".into());
        }
        if !component
            .navigate_parent()
            .map_err(|error| format!("return to parent through Files component: {error:?}"))?
        {
            return Err("component did not return to the parent location".into());
        }
        if !component.state().location.is_root() || component.state().breadcrumbs.len() != 1 {
            return Err("root location and breadcrumb were not restored".into());
        }

        let selected = component
            .state()
            .entries
            .iter()
            .find(|entry| entry.name.as_str() == FIXTURE_NAME && entry.kind == EntryKind::File)
            .ok_or("open fixture was absent after returning to root")?;
        let mut selected_id = selected.id;
        component
            .select(selected_id, false)
            .map_err(|error| format!("select file through Files component: {error:?}"))?;
        let (saved, save_operation) = component
            .save_selected(SAVED_CONTENTS.as_bytes(), Actor::User)
            .map_err(|error| format!("save selected file through Files component: {error:?}"))?;
        if save_operation.action_id != "files.write"
            || save_operation.outcome != OperationOutcome::Applied
            || save_operation.reversibility != Reversibility::Reversible
        {
            return Err("save operation result did not match files.write".into());
        }
        selected_id = saved.id;
        let (bytes, open_operation) = component
            .open_selected(Actor::User)
            .map_err(|error| format!("open selected file through Files component: {error:?}"))?
            .ok_or("open selected file returned no content")?;
        let opened_contents = String::from_utf8(bytes)
            .map_err(|_| "open selected file returned non-UTF-8 fixture content")?;
        if opened_contents != SAVED_CONTENTS {
            return Err("open output did not match the saved contents".into());
        }
        if open_operation.action_id != "files.open"
            || open_operation.outcome != OperationOutcome::Applied
            || open_operation.reversibility != Reversibility::NoMutation
        {
            return Err("open operation result did not match files.open".into());
        }

        let layout = component.state().layout();
        let inspector = layout
            .inspector
            .as_ref()
            .ok_or("selected file was absent from the inspector")?;
        if inspector.id != selected_id || !layout.selected.contains(&selected_id) {
            return Err("inspector and selection did not share the selected resource".into());
        }
        let mut canvas = GuestCanvas::default();
        component.draw(
            &mut canvas,
            FilesCanvasRect::new(0, 0, 640, 360),
            "This folder is empty.",
            Locale::EnUs,
        );
        let resource_text = format!("F: {FIXTURE_NAME}");
        if canvas.selected_rows != 1
            || !canvas.texts.iter().any(|text| text == &resource_text)
            || !canvas.texts.iter().any(|text| text == FIXTURE_NAME)
            || !canvas.texts.iter().any(|text| text == "Metadata")
        {
            return Err(
                "component canvas did not draw the selected provider item and inspector".into(),
            );
        }
        verify_read_only_save_feedback(&sandbox)?;
        let inspector_name = inspector.name.to_string();

        let copy_destination = created_folder.child_location();
        let copy_name = FileName::parse(COPIED_NAME)
            .map_err(|error| format!("copy name: {error:?}"))?;
        let (copied, copy_operation) = component
            .copy_selected(
                &copy_destination,
                &copy_name,
                Actor::User,
                &CancellationToken::new(),
            )
            .map_err(|error| format!("copy selected file through Files component: {error:?}"))?;
        if copied.location != copy_destination
            || copied.name.as_str() != COPIED_NAME
            || copy_operation.action_id != "files.copy"
            || copy_operation.outcome != OperationOutcome::Applied
        {
            return Err("Files component copy result did not match".into());
        }
        let copied_inspector = component.state().layout().inspector;
        if component.state().location != Location::root()
            || !component.state().selected.contains(&selected_id)
            || component.state().focused != Some(selected_id)
            || copied_inspector.as_ref().map(|entry| entry.id) != Some(selected_id)
        {
            return Err("copy changed the visible source selection or Inspector".into());
        }

        component
            .navigate(copy_destination.clone())
            .map_err(|error| format!("reload copy destination through Files component: {error:?}"))?;
        let copied_visible = component
            .state()
            .entries
            .iter()
            .find(|entry| entry.id == copied.id && entry.name.as_str() == COPIED_NAME)
            .ok_or("copied entry was absent after destination relisting")?;
        let copied_id = copied_visible.id;
        component
            .select(copied_id, false)
            .map_err(|error| format!("select copied file through Files component: {error:?}"))?;
        let (copy_bytes, copy_open_operation) = component
            .open_selected(Actor::User)
            .map_err(|error| format!("open copied file through Files component: {error:?}"))?
            .ok_or("open copied file returned no content")?;
        let copied_contents = String::from_utf8(copy_bytes)
            .map_err(|_| "open copied file returned non-UTF-8 fixture content")?;
        let copied_content_verified = copied_contents == SAVED_CONTENTS;
        if !copied_content_verified
            || copy_open_operation.action_id != "files.open"
            || copy_open_operation.outcome != OperationOutcome::Applied
        {
            return Err("reloaded copied content or operation result did not match".into());
        }

        let moved_name = FileName::parse(MOVED_NAME)
            .map_err(|error| format!("move name: {error:?}"))?;
        let (moved, move_operation) = component
            .move_selected(&Location::root(), &moved_name, Actor::User)
            .map_err(|error| format!("move selected file through Files component: {error:?}"))?;
        if moved.location != Location::root()
            || moved.name.as_str() != MOVED_NAME
            || move_operation.action_id != "files.move"
            || move_operation.outcome != OperationOutcome::Applied
        {
            return Err("Files component move result did not match".into());
        }
        if component.state().location != copy_destination
            || !component.state().entries.is_empty()
            || !component.state().selected.is_empty()
            || component.state().focused.is_some()
            || component.state().layout().inspector.is_some()
        {
            return Err("move did not clear the removed row, selection, and Inspector".into());
        }

        if !component
            .navigate_parent()
            .map_err(|error| format!("relist move destination through Files component: {error:?}"))?
        {
            return Err("component did not return from the move source folder".into());
        }
        let moved_visible = component
            .state()
            .entries
            .iter()
            .find(|entry| entry.id == moved.id && entry.name.as_str() == MOVED_NAME)
            .ok_or("moved entry was absent after destination relisting")?;
        let moved_id = moved_visible.id;
        component
            .select(moved_id, false)
            .map_err(|error| format!("select moved file through Files component: {error:?}"))?;
        let (moved_bytes, move_open_operation) = component
            .open_selected(Actor::User)
            .map_err(|error| format!("open moved file through Files component: {error:?}"))?
            .ok_or("open moved file returned no content")?;
        let moved_contents = String::from_utf8(moved_bytes)
            .map_err(|_| "open moved file returned non-UTF-8 fixture content")?;
        let moved_content_verified = moved_contents == SAVED_CONTENTS;
        if !moved_content_verified
            || move_open_operation.action_id != "files.open"
            || move_open_operation.outcome != OperationOutcome::Applied
        {
            return Err("reloaded moved content or operation result did not match".into());
        }
        if component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id)
            != Some(moved_id)
        {
            return Err(
                "moved file selection and Inspector did not share the same resource".into(),
            );
        }

        let (trashed, trash_operation) = component
            .trash_selected(Actor::User)
            .map_err(|error| format!("trash selected file through Files component: {error:?}"))?;
        if trashed.id != moved_id
            || trash_operation.action_id != "files.delete"
            || trash_operation.outcome != OperationOutcome::Applied
            || component
                .state()
                .entries
                .iter()
                .any(|entry| entry.id == moved_id)
            || !component.state().selected.is_empty()
            || component.state().focused.is_some()
            || component.state().layout().inspector.is_some()
        {
            return Err(
                "trash did not remove the selected item and Inspector from the listing".into(),
            );
        }
        let trash_items = component
            .list_trash()
            .map_err(|error| format!("list Files trash through component: {error:?}"))?;
        if trash_items.len() != 1 || trash_items[0].id != trashed.id {
            return Err("Files trash listing did not contain the selected item".into());
        }

        let (restored, restore_operation) = component
            .restore_trash_item(trash_items[0].id, Actor::User)
            .map_err(|error| format!("restore Files trash item through component: {error:?}"))?;
        if restored.id != moved_id
            || restored.name.as_str() != MOVED_NAME
            || restored.location != Location::root()
            || restore_operation.action_id != "files.restore"
            || restore_operation.outcome != OperationOutcome::Applied
            || !component
                .state()
                .entries
                .iter()
                .any(|entry| entry.id == moved_id)
            || !component.state().selected.contains(&moved_id)
            || component.state().focused != Some(moved_id)
            || component
                .state()
                .layout()
                .inspector
                .as_ref()
                .map(|entry| entry.id)
                != Some(moved_id)
        {
            return Err("restore did not reload and select the item in the Files listing".into());
        }
        let (restored_bytes, restored_open_operation) = component
            .open_selected(Actor::User)
            .map_err(|error| format!("open restored file through Files component: {error:?}"))?
            .ok_or("open restored file returned no content")?;
        let restored_contents = String::from_utf8(restored_bytes)
            .map_err(|_| "open restored file returned non-UTF-8 fixture content")?;
        let restored_content_verified = restored_contents == SAVED_CONTENTS;
        let trash_items_after_restore = component
            .list_trash()
            .map_err(|error| format!("verify Files trash after restore: {error:?}"))?
            .len();
        if !restored_content_verified
            || restored_open_operation.action_id != "files.open"
            || restored_open_operation.outcome != OperationOutcome::Applied
            || trash_items_after_restore != 0
        {
            return Err("restored file content or post-restore trash listing did not match".into());
        }

        let breadcrumb_path = component
            .state()
            .breadcrumbs
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>()
            .join("/");
        (
            reloaded_count,
            entered_breadcrumbs,
            breadcrumb_path,
            inspector_name,
            opened_contents,
            copied_content_verified,
            moved_content_verified,
            restored_content_verified,
            trash_items_after_restore,
        )
    };

    if initial_count != 1 || reloaded_count != 2 {
        return Err(format!(
            "unexpected visible entry counts: initial={initial_count}, reloaded={reloaded_count}"
        ));
    }

    print!("NAGI_FILES_OPENED_CONTENT {opened_contents}");
    println!(
        "NAGI_FILES_LINUX_GUEST_PASS component=true inputs=refresh,create-folder,reopen,navigate-folder,listing,parent,select,save-file,open-file,read-only-save-denied,denial-visible,open-clears-denial,copy-file,reload-copy,select-copy,open-copy,move-file,reload-move,select-moved,open-moved,trash-selected,trash-list,restore,restore-list,open-restored enumerated={initial_count} created={} reloaded={reloaded_count} opened_file={} content_verified=true read_only_save_denied=true denial_visible=true denial_cleared_after_open=true denied_save_preserved_bytes=true copied={} copied_content_verified={} moved={} moved_content_verified={} restored_content_verified={} trash_items_after_restore={} operations=files.write,files.open,files.copy,files.move,files.delete,files.restore fixture_scope=temporary_sandbox fixture_authority=CapabilitySet fixture_read=granted fixture_write=granted fixture_delete=granted fixture_restore=granted trash_listing_cleared=true restored_selection=true canvas_selected=true canvas_item=true canvas_inspector=true copy_source_selection_preserved=true moved_source_selection_cleared=true entered_breadcrumbs={} breadcrumbs={} inspector={} fixture_provider_reopened=true auth=not_connected ui=not_present",
        folder,
        FIXTURE_NAME,
        COPIED_NAME,
        copied_content_verified,
        MOVED_NAME,
        moved_content_verified,
        restored_content_verified,
        trash_items_after_restore,
        entered_breadcrumbs,
        breadcrumb_path,
        inspector_name
    );
    Ok(())
}

fn verify_read_only_save_feedback(sandbox: &Path) -> Result<(), String> {
    let mut component = open_read_only_component(sandbox)?;
    component
        .refresh()
        .map_err(|error| format!("read-only Files listing: {error:?}"))?;
    let entry = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == FIXTURE_NAME && entry.kind == EntryKind::File)
        .ok_or("read-only Files listing omitted the open fixture")?
        .clone();
    component
        .select(entry.id, false)
        .map_err(|error| format!("select read-only fixture file: {error:?}"))?;

    let error = component
        .save_selected(b"read-only save must not change contents\n", Actor::User)
        .expect_err("read-only Files component must reject save");
    if error.kind != FilesErrorKind::PermissionDenied {
        return Err(format!("read-only save returned {:?}", error.kind));
    }
    let bytes_after_denied_save = std::fs::read(sandbox.join(FIXTURE_NAME))
        .map_err(|error| format!("verify bytes after denied save: {error}"))?;
    if bytes_after_denied_save != SAVED_CONTENTS.as_bytes() {
        return Err("denied read-only save changed the original file bytes".into());
    }

    let permission_message = "You do not have permission to access this location.";
    let mut denied_canvas = GuestCanvas::default();
    component.draw(
        &mut denied_canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    if !denied_canvas
        .texts
        .iter()
        .any(|text| text == permission_message)
        || !denied_canvas
            .texts
            .iter()
            .any(|text| text == &format!("F: {FIXTURE_NAME}"))
    {
        return Err(
            "read-only save denial or selected file was absent from the Files canvas".into(),
        );
    }

    let (contents, operation) = component
        .open_selected(Actor::User)
        .map_err(|error| format!("next allowed read after denied save: {error:?}"))?
        .ok_or("selected file was not opened after the denied save")?;
    if contents != SAVED_CONTENTS.as_bytes() || operation.action_id != "files.open" {
        return Err("next allowed read did not return the unchanged original contents".into());
    }
    let mut recovered_canvas = GuestCanvas::default();
    component.draw(
        &mut recovered_canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    let bytes_after_open = std::fs::read(sandbox.join(FIXTURE_NAME))
        .map_err(|error| format!("verify bytes after allowed read: {error}"))?;
    if recovered_canvas
        .texts
        .iter()
        .any(|text| text == permission_message)
        || !recovered_canvas
            .texts
            .iter()
            .any(|text| text == &format!("F: {FIXTURE_NAME}"))
        || bytes_after_open != SAVED_CONTENTS.as_bytes()
    {
        return Err("Files canvas did not recover after the permitted read".into());
    }
    Ok(())
}

fn open_read_only_component(
    sandbox: &Path,
) -> Result<FilesComponent<SandboxProvider, CapabilitySet>, String> {
    let provider = SandboxProvider::new(sandbox)
        .map_err(|error| format!("open read-only sandbox provider: {error:?}"))?;
    let fixture_location =
        Location::parse(FIXTURE_NAME).map_err(|error| format!("fixture location: {error:?}"))?;
    let capabilities = CapabilitySet::from_grants([
        CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
        CapabilityGrant::allow(fixture_location, CapabilityRight::Read),
    ]);
    Ok(FilesComponent::new(
        FilesService::new(provider, capabilities),
        Location::root(),
    ))
}

fn open_component(
    sandbox: &Path,
    folder: &str,
) -> Result<FilesComponent<SandboxProvider, CapabilitySet>, String> {
    let provider = SandboxProvider::new(sandbox)
        .map_err(|error| format!("open sandbox provider: {error:?}"))?;
    let mut capabilities = CapabilitySet::new();
    capabilities.grant(CapabilityGrant::allow(
        Location::root(),
        CapabilityRight::Enumerate,
    ));
    capabilities.grant(CapabilityGrant::allow(
        Location::root(),
        CapabilityRight::Create,
    ));
    capabilities.grant(CapabilityGrant::allow(
        Location::parse(FIXTURE_NAME).map_err(|error| format!("fixture location: {error:?}"))?,
        CapabilityRight::Read,
    ));
    let copied_location = Location::parse(&format!("{folder}/{COPIED_NAME}"))
        .map_err(|error| format!("copied file location: {error:?}"))?;
    capabilities.grant(CapabilityGrant::allow(
        copied_location.clone(),
        CapabilityRight::Read,
    ));
    capabilities.grant(CapabilityGrant::allow(
        copied_location,
        CapabilityRight::Move,
    ));
    capabilities.grant(CapabilityGrant::allow(
        Location::parse(MOVED_NAME).map_err(|error| format!("moved file location: {error:?}"))?,
        CapabilityRight::Read,
    ));
    let moved_location =
        Location::parse(MOVED_NAME).map_err(|error| format!("moved file location: {error:?}"))?;
    capabilities.grant(CapabilityGrant::allow(
        moved_location.clone(),
        CapabilityRight::Enumerate,
    ));
    capabilities.grant(CapabilityGrant::allow(
        moved_location.clone(),
        CapabilityRight::Delete,
    ));
    capabilities.grant(CapabilityGrant::allow(
        moved_location,
        CapabilityRight::Restore,
    ));
    capabilities.grant(CapabilityGrant::allow(
        Location::parse(FIXTURE_NAME).map_err(|error| format!("fixture location: {error:?}"))?,
        CapabilityRight::Write,
    ));
    Ok(FilesComponent::new(
        FilesService::new(provider, capabilities),
        Location::root(),
    ))
}
