use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use nagi_model::ObjectId;
use nagi_notes::{
    Block, BlockKind, HostPreviewStore, Locale, Localizer, NoteSession, NoteStore, NotesApp,
    NotesSearchProvider, SearchProvider,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("nagi-notes-preview: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let mut root = None;
    let mut locale = Locale::EnUs;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--root" => root = arguments.next().map(PathBuf::from),
            "--locale" => {
                if let Some(value) = arguments.next() {
                    locale = Locale::parse(&value).unwrap_or(Locale::EnUs);
                }
            }
            "--help" | "-h" => {
                println!(
                    "Usage: nagi-notes-preview --root <sandbox-directory> [--locale en-US|ja-JP]"
                );
                return Ok(());
            }
            _ => return Err(format!("unknown option: {argument}").into()),
        }
    }
    let root = root.ok_or("provide an explicit --root sandbox directory")?;
    let store = Arc::new(HostPreviewStore::open(root)?);
    let store_trait: Arc<dyn NoteStore> = store;
    let app = Arc::new(NotesApp::new(store_trait));
    let search = NotesSearchProvider::for_app(Arc::clone(&app));
    let mut localizer = Localizer::new(locale);
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut open: Option<Arc<NoteSession>> = None;

    writeln!(stdout, "{}", localizer.text("message.host_preview"))?;
    writeln!(stdout, "{}", localizer.text("app.host_preview"))?;
    writeln!(stdout, "{}", localizer.text("action.help"))?;

    for input in stdin.lock().lines() {
        let input = input?;
        let input = input.trim_end_matches('\r');
        if input.trim().is_empty() {
            continue;
        }
        let (command, argument) = input
            .split_once(' ')
            .map(|(command, argument)| (command, argument.trim_start()))
            .unwrap_or((input, ""));
        match command {
            "help" => writeln!(stdout, "{}", localizer.text("action.help"))?,
            "lang" => match Locale::parse(argument) {
                Some(locale) => {
                    localizer.set_locale(locale);
                    writeln!(
                        stdout,
                        "{} {}",
                        localizer.text("message.language"),
                        locale.tag()
                    )?;
                }
                None => writeln!(stdout, "{}", localizer.text("error.invalid_command"))?,
            },
            "new" => {
                if argument.trim().is_empty() {
                    writeln!(stdout, "{}", localizer.text("error.title_required"))?;
                    continue;
                }
                close_open(&app, &mut open, &localizer)?;
                let session = app.create_note(argument)?;
                let note = session.flush()?;
                writeln!(
                    stdout,
                    "{} {:016x} {}",
                    localizer.text("message.created"),
                    note.id.0,
                    note.title
                )?;
                open = Some(session);
            }
            "quick" => {
                close_open(&app, &mut open, &localizer)?;
                let session =
                    app.create_quick_note(localizer.text("action.quick_title"), argument, None)?;
                let note = session.flush()?;
                writeln!(
                    stdout,
                    "{} {:016x} {}",
                    localizer.text("message.created"),
                    note.id.0,
                    note.title
                )?;
                open = Some(session);
            }
            "list" => {
                let notes = app.list_notes()?;
                if notes.is_empty() {
                    writeln!(stdout, "{}", localizer.text("empty.notes"))?;
                } else {
                    for note in notes {
                        writeln!(
                            stdout,
                            "{:016x}  r{}  {}",
                            note.id.0, note.revision, note.title
                        )?;
                    }
                }
            }
            "open" => match parse_id(argument) {
                Some(id) => {
                    close_open(&app, &mut open, &localizer)?;
                    let session = app.open_note(id)?;
                    writeln!(
                        stdout,
                        "{} {:016x} {}",
                        localizer.text("message.opened"),
                        id.0,
                        session.note().title
                    )?;
                    open = Some(session);
                }
                None => writeln!(stdout, "{}", localizer.text("error.invalid_id"))?,
            },
            "title" => match open.as_ref() {
                Some(session) if !argument.trim().is_empty() => session.set_title(argument)?,
                Some(_) => writeln!(stdout, "{}", localizer.text("error.title_required"))?,
                None => writeln!(stdout, "{}", localizer.text("error.no_open_note"))?,
            },
            "append" => match open.as_ref() {
                Some(session) => session.append_block(Block::new(
                    app.new_block_id(),
                    BlockKind::Paragraph(argument.to_owned()),
                ))?,
                None => writeln!(stdout, "{}", localizer.text("error.no_open_note"))?,
            },
            "show" => match open.as_ref() {
                Some(session) => {
                    let note = session.note();
                    let markdown = nagi_notes::export_markdown(&note);
                    if markdown.trim() == format!("# {}", note.title) {
                        writeln!(stdout, "{}", localizer.text("empty.note"))?;
                    } else {
                        write!(stdout, "{markdown}")?;
                    }
                }
                None => writeln!(stdout, "{}", localizer.text("error.no_open_note"))?,
            },
            "status" => match open.as_ref() {
                Some(session) => {
                    let note = session.note();
                    writeln!(
                        stdout,
                        "{:016x}  r{}  {}",
                        note.id.0,
                        note.revision,
                        status_text(session, &localizer)
                    )?;
                }
                None => writeln!(stdout, "{}", localizer.text("error.no_open_note"))?,
            },
            "save" => match open.as_ref() {
                Some(session) => match session.flush() {
                    Ok(note) => writeln!(
                        stdout,
                        "{} r{}",
                        localizer.text("message.saved"),
                        note.revision
                    )?,
                    Err(error) => {
                        eprintln!("save error: {error}");
                        writeln!(stdout, "{}", localizer.text("state.save_failed"))?;
                    }
                },
                None => writeln!(stdout, "{}", localizer.text("error.no_open_note"))?,
            },
            "retry" => match open.as_ref() {
                Some(session) => match session.retry() {
                    Ok(note) => writeln!(
                        stdout,
                        "{} r{}",
                        localizer.text("message.saved"),
                        note.revision
                    )?,
                    Err(error) => {
                        eprintln!("save retry error: {error}");
                        writeln!(stdout, "{}", localizer.text("state.save_failed"))?;
                    }
                },
                None => writeln!(stdout, "{}", localizer.text("error.no_open_note"))?,
            },
            "search" => {
                let hits = search.search(argument)?;
                if hits.is_empty() {
                    writeln!(stdout, "{}", localizer.text("message.search_empty"))?;
                } else {
                    writeln!(stdout, "{}", localizer.text("message.search_results"))?;
                    for hit in hits {
                        writeln!(stdout, "{:016x}  {}", hit.object_id.0, hit.snippet)?;
                    }
                }
            }
            "delete" => match parse_id(argument) {
                Some(id) => {
                    if open.as_ref().is_some_and(|session| session.note().id == id) {
                        open.take();
                    }
                    app.delete_note(id)?;
                    writeln!(stdout, "{}", localizer.text("message.deleted"))?;
                }
                None => writeln!(stdout, "{}", localizer.text("error.invalid_id"))?,
            },
            "restore" => match parse_id(argument) {
                Some(id) => {
                    app.restore_note(id)?;
                    writeln!(stdout, "{}", localizer.text("message.restored"))?;
                }
                None => writeln!(stdout, "{}", localizer.text("error.invalid_id"))?,
            },
            "close" => {
                if close_open(&app, &mut open, &localizer)? {
                    writeln!(stdout, "{}", localizer.text("message.closed"))?;
                }
            }
            "quit" | "exit" => {
                if close_open(&app, &mut open, &localizer)? {
                    break;
                }
            }
            _ => writeln!(stdout, "{}", localizer.text("error.invalid_command"))?,
        }
        stdout.flush()?;
    }
    close_open(&app, &mut open, &localizer)?;
    Ok(())
}

fn parse_id(input: &str) -> Option<ObjectId> {
    let input = input.trim();
    if input.len() != 16 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(input, 16).ok().map(ObjectId)
}

fn close_open(
    app: &NotesApp,
    open: &mut Option<Arc<NoteSession>>,
    localizer: &Localizer,
) -> Result<bool, Box<dyn std::error::Error>> {
    if let Some(session) = open.as_ref() {
        if let Err(error) = app.close_note(session.note().id) {
            eprintln!("close error: {error}");
            writeln!(io::stderr(), "{}", localizer.text("error.unsaved_close"))?;
            return Ok(false);
        }
    }
    *open = None;
    Ok(true)
}

fn status_text(session: &NoteSession, localizer: &Localizer) -> String {
    match session.status() {
        nagi_notes::SaveStatus::Saved | nagi_notes::SaveStatus::Closed => {
            localizer.text("state.saved").to_owned()
        }
        nagi_notes::SaveStatus::Dirty => localizer.text("state.dirty").to_owned(),
        nagi_notes::SaveStatus::Saving => localizer.text("state.saving").to_owned(),
        nagi_notes::SaveStatus::Failed(_) => localizer.text("state.save_failed").to_owned(),
    }
}
