use crate::{
    Actor, CapabilityAuthorizer, EntryKind, FileName, FilesApp, FilesError, FilesService,
    FilesystemProvider, OperationResult,
};

/// Dispatches the shared filesystem commands accepted by the host preview and
/// the Linux guest smoke consumer. `None` leaves the line to the preview's
/// remaining command handlers.
pub fn dispatch_input_line<P, A, F, C>(
    line: &str,
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    mut report_operation: F,
    mut display_opened_contents: C,
) -> Option<Result<(), FilesError>>
where
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
    F: FnMut(&OperationResult),
    C: FnMut(&str),
{
    let line = line.trim();
    let (command, rest) = line.split_once(' ').unwrap_or((line, ""));

    match command {
        "refresh" | "ls" => Some(app.refresh(service)),
        "mkdir" => Some(create_folder(service, app, rest, &mut report_operation)),
        "cd" => Some(
            resolve_location(&app.state.location, rest)
                .and_then(|location| app.navigate(service, location)),
        ),
        "up" => Some(app.navigate_parent(service).map(|_| ())),
        "select" => Some(select_item(app, rest, false)),
        "select-add" => Some(select_item(app, rest, true)),
        "open" => Some(open_item(
            service,
            app,
            rest,
            &mut report_operation,
            &mut display_opened_contents,
        )),
        _ => None,
    }
}

/// Handles selection movement that belongs only to the interactive host
/// preview, not to the shared preview and Linux guest-smoke command set.
pub fn dispatch_preview_only_line(
    line: &str,
    app: &mut FilesApp,
) -> Option<Result<(), FilesError>> {
    match line.trim() {
        "next" => Some(select_adjacent_item(app, true)),
        "previous" => Some(select_adjacent_item(app, false)),
        _ => None,
    }
}

fn select_adjacent_item(app: &mut FilesApp, forward: bool) -> Result<(), FilesError> {
    let entries = &app.state.entries;
    if entries.is_empty() {
        return Err(FilesError::new(crate::FilesErrorKind::NotFound));
    }

    let current = app
        .state
        .focused
        .filter(|id| app.state.selected.contains(id))
        .and_then(|focused| entries.iter().position(|entry| entry.id == focused));
    let index = match (forward, current) {
        (true, Some(index)) => (index + 1).min(entries.len() - 1),
        (true, None) => 0,
        (false, Some(index)) => index.saturating_sub(1),
        (false, None) => entries.len() - 1,
    };
    app.state.select(entries[index].id, false)
}

fn open_item<P, A, F, C>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
    report_operation: &mut F,
    display_opened_contents: &mut C,
) -> Result<(), FilesError>
where
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
    F: FnMut(&OperationResult),
    C: FnMut(&str),
{
    let entry = if value.is_empty() {
        let focused = app
            .state
            .focused
            .filter(|id| app.state.selected.contains(id))
            .ok_or_else(|| FilesError::new(crate::FilesErrorKind::NotFound))?;
        app.state
            .entries
            .iter()
            .find(|entry| entry.id == focused)
            .cloned()
            .ok_or_else(|| FilesError::new(crate::FilesErrorKind::NotFound))?
    } else {
        app.state
            .entries
            .get(parse_index(value, app.state.entries.len())?)
            .cloned()
            .ok_or_else(|| FilesError::new(crate::FilesErrorKind::NotFound))?
    };
    if entry.kind == EntryKind::Folder {
        return app.navigate(service, entry.child_location());
    }
    let (contents, result) = service.open_file(&entry, Actor::User)?;
    report_operation(&result);
    display_opened_contents(&format_open_contents(&contents));
    Ok(())
}

fn format_open_contents(contents: &[u8]) -> String {
    match std::str::from_utf8(contents) {
        Ok(text) => text.to_owned(),
        Err(_) => format!("Binary file ({} bytes).", contents.len()),
    }
}

/// Resolves a preview path relative to the current Files location.
pub fn resolve_location(
    base: &crate::Location,
    user_path: &str,
) -> Result<crate::Location, FilesError> {
    if user_path.is_empty() || user_path == "." {
        return Ok(base.clone());
    }
    if user_path.starts_with('/') || user_path.contains('\\') {
        return Err(FilesError::new(crate::FilesErrorKind::InvalidLocation));
    }
    let combined = if base.is_root() {
        user_path.to_owned()
    } else {
        format!("{}/{}", base.as_str(), user_path)
    };
    crate::Location::parse(&combined)
}

fn select_item(app: &mut FilesApp, value: &str, additive: bool) -> Result<(), FilesError> {
    let index = parse_index(value, app.state.entries.len())?;
    app.state.select(app.state.entries[index].id, additive)
}

pub fn parse_index(value: &str, length: usize) -> Result<usize, FilesError> {
    value
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(1))
        .filter(|index| *index < length)
        .ok_or_else(|| FilesError::new(crate::FilesErrorKind::NotFound))
}

fn create_folder<P, A, F>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
    report_operation: &mut F,
) -> Result<(), FilesError>
where
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
    F: FnMut(&OperationResult),
{
    let name = FileName::parse(value)?;
    let (_, result) = service.create_folder(&app.state.location, &name, Actor::User)?;
    report_operation(&result);
    app.refresh(service)
}
