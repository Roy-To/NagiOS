use std::io::{self, Write};
use std::path::PathBuf;

use nagi_files::localization::{self, Locale};
use nagi_files::{
    render_three_pane, Actor, CancellationToken, CapabilityGrant, CapabilityRight, CapabilitySet,
    FileName, FilesApp, FilesError, FilesErrorKind, FilesSearchProvider, FilesService, Location,
    SandboxProvider,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut root: Option<PathBuf> = None;
    let mut rights = Vec::new();
    let mut locale = Locale::EnUs;
    let mut allow_seen = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                println!(
                    "Nagi Files host preview\n\nUsage: nagi-files-preview <sandbox-root> --allow <rights> [--locale en-US|ja-JP]\nRights: read,enumerate,create,write,rename,move,delete,restore,permanent-delete\nThe sandbox is the only host directory this preview can access."
                );
                return Ok(());
            }
            "--allow" => {
                let value = args.next().ok_or("--allow needs a comma-separated value")?;
                rights = parse_rights(&value)?;
                allow_seen = true;
            }
            "--locale" => {
                let value = args.next().ok_or("--locale needs en-US or ja-JP")?;
                locale = Locale::parse(&value);
            }
            value if value.starts_with('-') => return Err(format!("unknown option: {value}")),
            value if root.is_none() => root = Some(PathBuf::from(value)),
            value => return Err(format!("unexpected argument: {value}")),
        }
    }
    if !allow_seen {
        return Err("choose explicit sandbox rights with --allow".into());
    }
    let root = root.ok_or("provide an explicit sandbox root")?;
    let provider = SandboxProvider::new(&root)
        .map_err(|error| format!("sandbox unavailable: {}", localized_error(locale, &error)))?;
    let capabilities = CapabilitySet::from_grants(
        rights
            .into_iter()
            .map(|right| CapabilityGrant::allow(Location::root(), right)),
    );
    let mut service = FilesService::new(provider, capabilities);
    let mut app = FilesApp::new(Location::root());
    app.refresh(&service)
        .map_err(|error| localized_error(locale, &error))?;
    let search = FilesSearchProvider::default();

    println!(
        "{} — sandbox: {} (host preview; Nagi target integration: not run)",
        localization::text(locale, "files.title").unwrap_or("Files"),
        service.provider().sandbox_root().display()
    );
    println!("Only the explicit --allow rights are granted inside this sandbox.");
    print_help();
    loop {
        render(&app, locale);
        print!("{}> ", app.state.location);
        io::stdout().flush().map_err(|error| error.to_string())?;
        let mut line = String::new();
        if io::stdin()
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
        let result = match command {
            "quit" | "exit" => break,
            "help" => {
                print_help();
                continue;
            }
            "refresh" | "ls" => app.refresh(&service),
            "up" => app.navigate_parent(&service).map(|_| ()),
            "cd" => resolve_location(&app.state.location, rest)
                .and_then(|path| app.navigate(&service, path)),
            "select" | "select-add" => select_item(&mut app, rest, command == "select-add"),
            "clear" => {
                app.state.clear_selection();
                Ok(())
            }
            "toggle-inspector" => {
                app.state.inspector_visible = !app.state.inspector_visible;
                Ok(())
            }
            "mkdir" => create_folder(&mut service, &mut app, rest),
            "rename" => rename_item(&mut service, &mut app, rest),
            "copy" => copy_item(&mut service, &mut app, rest),
            "move" => move_item(&mut service, &mut app, rest),
            "duplicate" => duplicate_item(&mut service, &mut app, rest),
            "trash" => trash_item(&mut service, &mut app, rest),
            "trash-list" => print_trash(&service),
            "restore" => restore_item(&mut service, &mut app, rest),
            "delete-permanently" => permanently_delete(&mut service, &mut app, rest, locale),
            "tags" => set_tags(&mut service, &mut app, rest),
            "open" => open_item(&mut service, &mut app, rest),
            "search" => print_search(&search, &service, &app, rest),
            "context" => print_context(&app),
            _ => Err(FilesError::new(FilesErrorKind::InvalidLocation)),
        };
        if let Err(error) = result {
            println!("{}", localized_error(locale, &error));
        }
    }
    Ok(())
}

fn parse_rights(value: &str) -> Result<Vec<CapabilityRight>, String> {
    let mut rights = Vec::new();
    for item in value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let right = match item {
            "read" => CapabilityRight::Read,
            "enumerate" => CapabilityRight::Enumerate,
            "create" => CapabilityRight::Create,
            "write" => CapabilityRight::Write,
            "rename" => CapabilityRight::Rename,
            "move" => CapabilityRight::Move,
            "delete" => CapabilityRight::Delete,
            "restore" => CapabilityRight::Restore,
            "permanent-delete" => CapabilityRight::PermanentDelete,
            "metadata" => CapabilityRight::SetMetadata,
            "workspace-reference" => CapabilityRight::WorkspaceReference,
            other => return Err(format!("unknown capability right: {other}")),
        };
        if !rights.contains(&right) {
            rights.push(right);
        }
    }
    Ok(rights)
}

fn render(app: &FilesApp, locale: Locale) {
    let layout = app.state.layout();
    let here = app
        .state
        .breadcrumbs
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>()
        .join(" / ");
    println!("\n{here}");
    println!(
        "{}",
        render_three_pane(
            &layout,
            localization::text(locale, "files.empty").unwrap_or("This folder is empty."),
            locale
        )
    );
}

fn print_help() {
    println!(
        "Commands: ls | cd <relative-path> | up | select <n> | select-add <n> | clear | toggle-inspector\n\
         mkdir <name> | rename <n> <name> | copy <n> <destination> [name]\n\
         move <n> <destination> [name] | duplicate <n> | trash <n> | trash-list\n\
         restore <trash-index> | delete-permanently <trash-index> | tags <n> <tag,tag>\n\
         open <n> | search <text> | context | help | quit"
    );
}

fn select_item(app: &mut FilesApp, value: &str, additive: bool) -> Result<(), FilesError> {
    let index = parse_index(value, app.state.entries.len())?;
    app.state.select(app.state.entries[index].id, additive)
}

fn create_folder<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
) -> Result<(), FilesError> {
    let name = FileName::parse(value)?;
    let (_, result) = service.create_folder(&app.state.location, &name, Actor::User)?;
    report_operation(&result);
    app.refresh(service)
}

fn rename_item<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
) -> Result<(), FilesError> {
    let (index, name) = value
        .split_once(' ')
        .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidName))?;
    let entry = app
        .state
        .entries
        .get(parse_index(index, app.state.entries.len())?)
        .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?
        .clone();
    let (_, result) = service.rename(&entry, &FileName::parse(name)?, Actor::User)?;
    report_operation(&result);
    app.refresh(service)
}

fn copy_item<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
) -> Result<(), FilesError> {
    let mut parts = value.splitn(3, ' ');
    let index = parts.next().unwrap_or("");
    let destination = resolve_location(&app.state.location, parts.next().unwrap_or(""))?;
    let name = match parts.next() {
        Some(name) => FileName::parse(name)?,
        None => app.state.entries[parse_index(index, app.state.entries.len())?]
            .name
            .clone(),
    };
    let entry = app
        .state
        .entries
        .get(parse_index(index, app.state.entries.len())?)
        .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?
        .clone();
    let (_, result) = service.copy(
        &entry,
        &destination,
        &name,
        Actor::User,
        &CancellationToken::new(),
    )?;
    report_operation(&result);
    app.refresh(service)
}

fn move_item<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
) -> Result<(), FilesError> {
    let mut parts = value.splitn(3, ' ');
    let index = parts.next().unwrap_or("");
    let destination = resolve_location(&app.state.location, parts.next().unwrap_or(""))?;
    let entry = app
        .state
        .entries
        .get(parse_index(index, app.state.entries.len())?)
        .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?
        .clone();
    let name = match parts.next() {
        Some(name) => FileName::parse(name)?,
        None => entry.name.clone(),
    };
    let (_, result) = service.move_item(&entry, &destination, &name, Actor::User)?;
    report_operation(&result);
    app.refresh(service)
}

fn duplicate_item<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
) -> Result<(), FilesError> {
    let entry = app
        .state
        .entries
        .get(parse_index(value, app.state.entries.len())?)
        .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?
        .clone();
    let (_, result) = service.duplicate(&entry, Actor::User, &CancellationToken::new())?;
    report_operation(&result);
    app.refresh(service)
}

fn trash_item<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
) -> Result<(), FilesError> {
    let entry = app
        .state
        .entries
        .get(parse_index(value, app.state.entries.len())?)
        .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?
        .clone();
    let (_, result) = service.trash(&entry, Actor::User)?;
    report_operation(&result);
    app.refresh(service)
}

fn print_trash<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &FilesService<P, A>,
) -> Result<(), FilesError> {
    for (index, item) in service.list_trash()?.iter().enumerate() {
        println!(
            "{}  {} ({:?})",
            index + 1,
            item.original_location,
            item.kind
        );
    }
    Ok(())
}

fn set_tags<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
) -> Result<(), FilesError> {
    let (index, values) = value
        .split_once(' ')
        .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidName))?;
    let entry = app
        .state
        .entries
        .get(parse_index(index, app.state.entries.len())?)
        .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?
        .clone();
    let tags = values
        .split(',')
        .map(nagi_files::Tag::parse)
        .collect::<Result<Vec<_>, _>>()?;
    let (_, result) = service.set_tags(&entry, &tags, Actor::User)?;
    report_operation(&result);
    app.refresh(service)
}

fn restore_item<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
) -> Result<(), FilesError> {
    let trash = service.list_trash()?;
    let item = trash
        .get(parse_index(value, trash.len())?)
        .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
    let (_, result) = service.restore(item.id, Actor::User)?;
    report_operation(&result);
    app.refresh(service)
}

fn permanently_delete<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
    locale: Locale,
) -> Result<(), FilesError> {
    let trash = service.list_trash()?;
    let item = trash
        .get(parse_index(value, trash.len())?)
        .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
    let challenge = service.request_permanent_delete(item.id)?;
    print!(
        "{} [y/N] ",
        localization::text(locale, "files.confirm.permanent_delete")
            .unwrap_or("Delete permanently?")
    );
    io::stdout()
        .flush()
        .map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure))?;
    let mut confirmation = String::new();
    io::stdin()
        .read_line(&mut confirmation)
        .map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure))?;
    if !matches!(
        confirmation.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ) {
        return Err(FilesError::new(FilesErrorKind::Cancelled));
    }
    let result = service.confirm_permanent_delete(challenge, Actor::User)?;
    report_operation(&result);
    app.refresh(service)
}

fn open_item<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    value: &str,
) -> Result<(), FilesError> {
    let entry = app
        .state
        .entries
        .get(parse_index(value, app.state.entries.len())?)
        .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
    if entry.kind == nagi_files::EntryKind::Folder {
        return app.navigate(service, entry.child_location());
    }
    let entry = entry.clone();
    let (contents, result) = service.open_file(&entry, Actor::User)?;
    report_operation(&result);
    match std::str::from_utf8(&contents) {
        Ok(text) => println!("{text}"),
        Err(_) => println!("Binary file ({} bytes).", contents.len()),
    }
    Ok(())
}

fn print_context(app: &FilesApp) -> Result<(), FilesError> {
    let context = app.context_snapshot();
    println!("Current location: {}", context.current_location);
    println!("Selected resources: {:?}", context.selected_resources);
    println!("Focused resource: {:?}", context.focused_resource);
    Ok(())
}

fn print_search<P: nagi_files::FilesystemProvider, A: nagi_files::CapabilityAuthorizer>(
    search: &FilesSearchProvider,
    service: &FilesService<P, A>,
    app: &FilesApp,
    query: &str,
) -> Result<(), FilesError> {
    for record in search.search(service, &app.state.location, query, 100)? {
        println!(
            "{:>3}  {}  ({:?}, {})",
            record.resource_id,
            record.location,
            record.kind,
            record
                .size_bytes
                .map_or("—".to_owned(), |n| format!("{n} B"))
        );
    }
    Ok(())
}

fn resolve_location(base: &Location, user_path: &str) -> Result<Location, FilesError> {
    if user_path.is_empty() || user_path == "." {
        return Ok(base.clone());
    }
    if user_path.starts_with('/') || user_path.contains('\\') {
        return Err(FilesError::new(FilesErrorKind::InvalidLocation));
    }
    let combined = if base.is_root() {
        user_path.to_owned()
    } else {
        format!("{}/{}", base.as_str(), user_path)
    };
    Location::parse(&combined)
}

fn parse_index(value: &str, length: usize) -> Result<usize, FilesError> {
    let index = value
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(1))
        .filter(|index| *index < length)
        .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
    Ok(index)
}

fn report_operation(result: &nagi_files::OperationResult) {
    println!(
        "{} applied (transaction {}). Activity: {:?}; checkpoint: {:?}; reversible: {:?}",
        result.action_id,
        result.transaction_id.0,
        result.activity,
        result.checkpoint,
        result.reversibility
    );
}

fn localized_error(locale: Locale, error: &FilesError) -> String {
    let key = match error.kind {
        FilesErrorKind::PermissionDenied | FilesErrorKind::PermissionRequired => {
            "files.error.permission"
        }
        FilesErrorKind::ProviderUnavailable | FilesErrorKind::CapabilityUnavailable => {
            "files.error.unavailable"
        }
        FilesErrorKind::Conflict | FilesErrorKind::AlreadyExists => "files.error.conflict",
        FilesErrorKind::SandboxEscape => "files.error.sandbox_escape",
        FilesErrorKind::SymlinkNotAllowed => "files.error.symlink",
        FilesErrorKind::ConfirmationLimitReached => "files.error.confirmation_limit",
        FilesErrorKind::Cancelled => "files.error.cancelled",
        FilesErrorKind::ActivityUnavailable | FilesErrorKind::ActivityFailure => {
            "files.error.activity_unavailable"
        }
        FilesErrorKind::WorkspaceUnavailable => "files.error.workspace_unavailable",
        FilesErrorKind::DestinationInsideSource => "files.error.destination_inside_source",
        FilesErrorKind::InvalidName => "files.error.invalid_name",
        FilesErrorKind::NotDirectory => "files.error.not_directory",
        FilesErrorKind::FileTooLarge => "files.error.preview_too_large",
        FilesErrorKind::InvalidLocation => "files.error.invalid_location",
        _ => "files.error.invalid_location",
    };
    localization::text(locale, key)
        .unwrap_or("The operation failed.")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{localized_error, Locale};
    use nagi_files::{FilesError, FilesErrorKind};

    #[test]
    fn confirmation_limit_error_has_english_and_japanese_messages() {
        let error = FilesError::new(FilesErrorKind::ConfirmationLimitReached);

        assert_eq!(
            localized_error(Locale::EnUs, &error),
            "Too many permanent-delete confirmations are pending. Cancel or complete one before trying again."
        );
        assert_eq!(
            localized_error(Locale::JaJp, &error),
            "完全削除の確認が保留中の上限に達しました。既存の確認を取り消すか完了してから、もう一度お試しください。"
        );
    }
}
