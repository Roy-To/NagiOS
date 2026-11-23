//! Pure command-line contract for the opt-in normal PID 1 System/Data probe.

const PROBE_ARGUMENT: &str = "nagi.dev_system_data.probe=";

const POWEROFF_ARGUMENT: &str = "nagi.dev_system_data.exit=poweroff";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ProbeRequest {
    token: String,
    poweroff: bool,
}

impl ProbeRequest {
    pub(crate) fn run<T, E>(
        &self,
        observe: impl FnOnce(&str) -> Result<T, E>,
        report: impl FnOnce(T),
        poweroff: impl FnOnce(),
    ) -> Result<(), E> {
        report(observe(&self.token)?);
        if self.poweroff {
            poweroff();
        }
        Ok(())
    }
}

pub(crate) fn parse_probe_request<'a>(
    arguments: impl Iterator<Item = &'a str>,
) -> Result<Option<ProbeRequest>, &'static str> {
    let mut selected = None;
    let mut poweroff = false;
    for argument in arguments {
        if let Some(value) = argument.strip_prefix(PROBE_ARGUMENT) {
            if selected.is_some() {
                return Err("system_data_probe_token_duplicate");
            }
            if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("system_data_probe_token_malformed");
            }
            selected = Some(value.to_owned());
        } else if argument == POWEROFF_ARGUMENT {
            if poweroff {
                return Err("system_data_probe_exit_duplicate");
            }
            poweroff = true;
        } else if argument.starts_with("nagi.dev_system_data.") {
            return Err("system_data_probe_argument_unknown");
        }
    }
    if poweroff && selected.is_none() {
        return Err("system_data_probe_exit_without_token");
    }
    Ok(selected.map(|token| ProbeRequest { token, poweroff }))
}

#[cfg(test)]
mod tests {
    use super::{parse_probe_request, ProbeRequest, POWEROFF_ARGUMENT};

    #[test]
    fn accepts_one_exact_probe_token_and_ignores_unrelated_arguments() {
        let arguments = [
            "console=ttyS0",
            "nagi.dev_system_data.probe=0123456789abcdef0123456789abcdef",
        ];
        assert_eq!(
            parse_probe_request(arguments.into_iter()).unwrap(),
            Some(ProbeRequest {
                token: "0123456789abcdef0123456789abcdef".to_owned(),
                poweroff: false,
            })
        );
        assert_eq!(
            parse_probe_request(["console=ttyS0"].into_iter()).unwrap(),
            None
        );
    }

    #[test]
    fn rejects_duplicate_malformed_and_unknown_probe_arguments() {
        let token = "nagi.dev_system_data.probe=0123456789abcdef0123456789abcdef";
        assert_eq!(
            parse_probe_request([token, token].into_iter()).unwrap_err(),
            "system_data_probe_token_duplicate"
        );
        assert_eq!(
            parse_probe_request(["nagi.dev_system_data.probe=not-a-token"].into_iter())
                .unwrap_err(),
            "system_data_probe_token_malformed"
        );
        assert_eq!(
            parse_probe_request(["nagi.dev_system_data.other=value"].into_iter()).unwrap_err(),
            "system_data_probe_argument_unknown"
        );
    }

    #[test]
    fn poweroff_requires_one_explicit_argument_and_a_probe_token() {
        let token = "nagi.dev_system_data.probe=0123456789abcdef0123456789abcdef";
        assert!(
            parse_probe_request([POWEROFF_ARGUMENT, token].into_iter())
                .unwrap()
                .unwrap()
                .poweroff
        );
        assert_eq!(
            parse_probe_request([POWEROFF_ARGUMENT].into_iter()),
            Err("system_data_probe_exit_without_token")
        );
        assert_eq!(
            parse_probe_request([token, POWEROFF_ARGUMENT, POWEROFF_ARGUMENT].into_iter()),
            Err("system_data_probe_exit_duplicate")
        );
    }

    #[test]
    fn only_successful_opt_in_observation_reports_then_powers_off() {
        use std::cell::RefCell;
        let events = RefCell::new(Vec::new());
        let mut request = ProbeRequest {
            token: "fixture".to_owned(),
            poweroff: true,
        };
        assert_eq!(
            request.run(
                |_| Err::<(), _>("observation failed"),
                |_| panic!("failed observation must not report success"),
                || panic!("failed observation must not power off"),
            ),
            Err("observation failed"),
        );
        for poweroff in [true, false] {
            request.poweroff = poweroff;
            events.borrow_mut().clear();
            request
                .run(
                    |token| {
                        assert_eq!(token, "fixture");
                        Ok::<_, ()>("observed")
                    },
                    |observation| events.borrow_mut().push(observation),
                    || events.borrow_mut().push("poweroff"),
                )
                .unwrap();
            let expected: &[_] = if poweroff {
                &["observed", "poweroff"]
            } else {
                &["observed"]
            };
            assert_eq!(events.borrow().as_slice(), expected);
        }
    }
}
