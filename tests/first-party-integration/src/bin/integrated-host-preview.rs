use nagi_files::{Actor as FilesActor, FileName, Location};
use nagi_first_party_integration::{insert_preview_file, IntegratedHost};
use nagi_home_search::{Locale, TypedAction};
use nagi_notes::{Block, BlockKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = IntegratedHost::new()?;
    let note = host.notes.create_note("Integration note · 連携ノート")?;
    note.append_block(Block::new(
        host.notes.new_block_id(),
        BlockKind::Paragraph("searchable integration record · 横断検索対象".to_owned()),
    ))?;
    let saved_note = note.flush()?;

    let (resource_id, rename_operation, folder_operation) = {
        let mut files = host
            .files
            .lock()
            .map_err(|_| "Files preview lock poisoned")?;
        let resource_id = insert_preview_file(
            files.provider_mut(),
            "Integration invoice.txt",
            b"host sandbox document",
        )?;
        let entry = files.metadata(&Location::parse("Integration invoice.txt")?)?;
        let (_, rename_operation) = files.rename(
            &entry,
            &FileName::parse("Integration invoice renamed.txt")?,
            FilesActor::User,
        )?;
        let (_, folder_operation) = files.create_folder(
            &Location::root(),
            &FileName::parse("Activity recorded")?,
            FilesActor::User,
        )?;
        (resource_id, rename_operation, folder_operation)
    };

    let english_apps = host
        .home_apps(Locale::EnUs)
        .map_err(|error| format!("Home snapshot failed: {error:?}"))?;
    let japanese_apps = host
        .home_apps(Locale::JaJp)
        .map_err(|error| format!("Home snapshot failed: {error:?}"))?;
    let english_search = host.search("invoice", Locale::EnUs)?;
    let japanese_search = host.search("連携", Locale::JaJp)?;
    let activity_search = host.search("Created", Locale::EnUs)?;
    let wayback_search = host.search("チェックポイント", Locale::JaJp)?;
    let (activity_count, checkpoint_count, revision_count) = host.history.counts()?;
    let resource_action = english_search
        .results
        .iter()
        .filter_map(|result| result.action.clone())
        .find(|action| matches!(action, TypedAction::OpenObject { .. }));
    let event_action = activity_search
        .results
        .iter()
        .filter_map(|result| result.action.clone())
        .find(|action| matches!(action, TypedAction::OpenActivityEvent { .. }));
    let checkpoint_action = wayback_search
        .results
        .iter()
        .filter_map(|result| result.action.clone())
        .find(|action| matches!(action, TypedAction::OpenCheckpoint { .. }));

    assert!(english_apps.contains(&"Notes".to_owned()));
    assert!(english_apps.contains(&"Files".to_owned()));
    assert!(japanese_apps.contains(&"ノート".to_owned()));
    assert!(japanese_apps.contains(&"ファイル".to_owned()));
    assert!(saved_note.revision > 0);
    assert!(resource_action.is_some_and(|action| matches!(action, TypedAction::OpenObject { .. })));
    assert!(
        event_action.is_some_and(|action| matches!(action, TypedAction::OpenActivityEvent { .. }))
    );
    assert!(checkpoint_action
        .is_some_and(|action| matches!(action, TypedAction::OpenCheckpoint { .. })));
    assert!(activity_search.provider_issues.is_empty());
    assert!(wayback_search.provider_issues.is_empty());

    println!("Nagi first-party integrated host preview (memory-only; target NOT RUN)");
    println!("Home apps en-US: {}", english_apps.join(", "));
    println!("Home apps ja-JP: {}", japanese_apps.join(", "));
    println!(
        "Search en-US 'invoice': {} result(s), typed file action present",
        english_search.results.len()
    );
    println!(
        "Search ja-JP '連携': {} result(s), Notes provider active",
        japanese_search.results.len()
    );
    println!(
        "Activity 'Created': {} result(s), typed event action present",
        activity_search.results.len()
    );
    println!(
        "Wayback 'チェックポイント': {} result(s), typed checkpoint action present",
        wayback_search.results.len()
    );
    println!("Shared Activity={activity_count}, Wayback checkpoints={checkpoint_count}, revisions={revision_count}");
    println!(
        "Files rename activity={:?}, checkpoint={:?}",
        rename_operation.activity, rename_operation.checkpoint
    );
    println!(
        "Files folder-create checkpoint={:?} (no pre-existing object to snapshot)",
        folder_operation.checkpoint
    );
    println!(
        "Files ResourceId retained in resolver: {}",
        host.files_objects
            .resource_id(host.files_objects.resolve(resource_id)?)?
            .is_some()
    );
    println!("Sandbox boundary: all Notes and Files data came from in-memory preview providers.");
    Ok(())
}
