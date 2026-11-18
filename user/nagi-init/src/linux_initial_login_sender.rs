use libnagi::credential::valid_name;
use libnagi::login::{LoginAction, LoginForm};
use libnagi::InputEvent;
use nagi_pid1_live_peer_supplier::initial_login::{
    send_initial_login_request_until, InitialLoginRequestMetadata, LoginWireError,
};
use nagi_pid1_live_peer_supplier::pid1_ready::{ChildServiceBootstrap, EndpointBootstrapError};
use std::ffi::OsString;
use std::io;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::Instant;

pub(super) enum UnlockEvent<E> {
    Ignored,
    Changed,
    Submission(Result<(), E>),
}

/// The Linux screen's unlock input for one broker connection.
pub(super) struct BrokerUnlock<'account> {
    input: UnlockInput<'account>,
    stream: UnixStream,
    metadata: InitialLoginRequestMetadata,
    deadline: Instant,
}

#[derive(Debug)]
pub(super) enum BrokerUnlockStartError {
    InvalidAccount,
    DeadlineExpired,
    Endpoint(EndpointBootstrapError),
    Connect(io::Error),
}

impl<'account> BrokerUnlock<'account> {
    /// Use the endpoint locator delivered to the login child by Task24.
    pub(super) fn from_task24_args(
        account_name: &'account [u8],
        args: &[OsString],
        metadata: InitialLoginRequestMetadata,
        deadline: Instant,
    ) -> Result<Self, BrokerUnlockStartError> {
        if Instant::now() >= deadline {
            return Err(BrokerUnlockStartError::DeadlineExpired);
        }
        let input = UnlockInput::new(account_name).ok_or(BrokerUnlockStartError::InvalidAccount)?;
        let endpoint = ChildServiceBootstrap::required_from_args(args)
            .map_err(BrokerUnlockStartError::Endpoint)?;
        let stream = endpoint
            .connect()
            .map_err(BrokerUnlockStartError::Connect)?;
        Ok(Self {
            input,
            stream,
            metadata,
            deadline,
        })
    }

    pub(super) fn handle_event(&mut self, event: InputEvent) -> UnlockEvent<LoginWireError> {
        let outcome = self.input.handle_event(event, |account_name, password| {
            let account_name =
                std::str::from_utf8(account_name).map_err(|_| LoginWireError::InvalidUtf8)?;
            let password =
                std::str::from_utf8(password).map_err(|_| LoginWireError::InvalidUtf8)?;
            send_initial_login_request_until(
                &mut self.stream,
                self.metadata,
                account_name,
                password,
                self.deadline,
            )
        });
        // The codec closes after a failed write. Also close on preflight
        // errors so the broker does not wait for a request that cannot arrive.
        if matches!(outcome, UnlockEvent::Submission(Err(_))) {
            let _ = self.stream.shutdown(Shutdown::Both);
        }
        outcome
    }
}

struct UnlockInput<'account> {
    account_name: &'account [u8],
    form: LoginForm,
    submitted: bool,
}

impl<'account> UnlockInput<'account> {
    fn new(account_name: &'account [u8]) -> Option<Self> {
        valid_name(account_name).then(|| Self {
            account_name,
            form: LoginForm::unlock(),
            submitted: false,
        })
    }

    fn handle_event<E>(
        &mut self,
        event: InputEvent,
        submit: impl FnOnce(&[u8], &[u8]) -> Result<(), E>,
    ) -> UnlockEvent<E> {
        if self.submitted || event.event_type != libnagi::INPUT_EVENT_KEY {
            return UnlockEvent::Ignored;
        }
        match self.form.handle_key(event.code, event.value != 0) {
            LoginAction::Unlock => {
                // A failed send may have written part of the request.
                self.submitted = true;
                let password = ClearPassword(&mut self.form);
                UnlockEvent::Submission(submit(self.account_name, password.0.password()))
            }
            LoginAction::Changed => UnlockEvent::Changed,
            LoginAction::Ignored | LoginAction::Create => UnlockEvent::Ignored,
        }
    }
}

struct ClearPassword<'form>(&'form mut LoginForm);

impl Drop for ClearPassword<'_> {
    fn drop(&mut self) {
        self.0.clear_secrets();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: u16) -> InputEvent {
        InputEvent {
            event_type: libnagi::INPUT_EVENT_KEY,
            code,
            value: 1,
        }
    }

    #[test]
    fn transport_error_clears_secret_and_closes_submission() {
        let mut client = UnlockInput::new(b"owner").unwrap();
        let unexpected_submit =
            |_: &[u8], _: &[u8]| -> Result<(), ()> { panic!("unexpected submission") };

        assert!(matches!(
            client.handle_event(key(libnagi::INPUT_KEY_ENTER), unexpected_submit),
            UnlockEvent::Ignored
        ));
        assert!(matches!(
            client.handle_event(key(30), unexpected_submit),
            UnlockEvent::Changed
        ));
        let submission = client.handle_event(key(libnagi::INPUT_KEY_ENTER), |account, password| {
            assert!(account == b"owner");
            assert!(password == b"a");
            Err(())
        });
        assert!(matches!(submission, UnlockEvent::Submission(Err(()))));
        assert!(client.form.password().is_empty());
        assert!(matches!(
            client.handle_event(key(libnagi::INPUT_KEY_ENTER), unexpected_submit),
            UnlockEvent::Ignored
        ));
    }
}
