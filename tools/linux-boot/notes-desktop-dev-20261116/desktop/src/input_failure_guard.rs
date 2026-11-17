use std::fmt::Display;

/// The caller must drop the owner only after a successful flush. A failed flush
/// returns the owner so the caller can keep the dirty session alive.
pub enum InputFailureAction<O, E> {
    ExitWithInputErrorAfterSave {
        owner: O,
        input_error: E,
    },
    HoldUnsavedDraft {
        owner: O,
        input_error: E,
        save_error: String,
        notification_error: Option<String>,
    },
}

/// Call the caller's final-flush check once before it decides whether it can
/// release the owner. The Notes session can short-circuit an already-failed
/// status; this helper never retries or discards the draft.
pub fn guard_input_failure<O, E>(
    mut owner: O,
    input_error: E,
    flush: impl FnOnce(&mut O) -> Result<(), String>,
    notify: impl FnOnce(&mut O, &str) -> Result<(), String>,
) -> InputFailureAction<O, E>
where
    E: Display,
{
    match flush(&mut owner) {
        Ok(()) => InputFailureAction::ExitWithInputErrorAfterSave { owner, input_error },
        Err(save_error) => {
            let notice = format!(
                "NAGI_NOTES_INPUT_SAVE_FAILED input={input_error} save={save_error} unsaved=1"
            );
            let notification_error = notify(&mut owner, &notice).err();
            InputFailureAction::HoldUnsavedDraft {
                owner,
                input_error,
                save_error,
                notification_error,
            }
        }
    }
}
