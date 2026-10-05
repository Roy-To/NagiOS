const REQUIRED_HTTPS_HOSTS: [&str; 3] = ["example.com", "example.org", "example.net"];
const TLS_PASS_PREFIX: &str = "Nagi M18 HTTPS TLS PASS ";
const CHROME_PRESENTED_PREFIX: &str = "Nagi M18 browser chrome PRESENTED ";
const PAGE_RENDERED_PREFIX: &str = "Nagi M18 HTTPS page RENDERED ";
const INPUT_NAVIGATION_LINE: &str = "Nagi M18 browser input navigation PASS host=example.com";
const SUMMARY_LINE: &str = "Nagi M18 browser scenario complete pages=3";
/// Minimum non-background pixels in each Servo page frame. A page that
/// painted only its background (for example with no usable font) has none.
const MIN_INK_PIXELS: u32 = 200;
/// Clipboard evidence, required in this order after the HTTPS pages and
/// before the summary.
/// The embedder-loaded fixture must reach the chrome as content navigation
/// before the clipboard steps.
const CONTENT_NAVIGATION_LINE: &str = "Nagi M18 browser content navigation PASS";
const CLIPBOARD_UNGESTURED_DENIAL: &str =
    "Nagi M18 clipboard ungestured read DENIED reason=no-user-gesture";
const CLIPBOARD_LINES: [&str; 5] = [
    "Nagi M18 clipboard page READY",
    "Nagi M18 clipboard copy observed",
    "Nagi M18 clipboard destination focused",
    CLIPBOARD_UNGESTURED_DENIAL,
    "Nagi M18 clipboard copy/paste PASS",
];
/// IME evidence, required in this order after the clipboard evidence.
const IME_LINES: [&str; 2] = ["Nagi M18 IME page READY", "Nagi M18 IME commit PASS"];
/// Upload evidence through Albert's trusted file picker, after the IME.
const UPLOAD_LINES: [&str; 3] = [
    "Nagi M18 upload page READY",
    "Nagi M18 upload picker READY",
    "Nagi M18 upload PASS",
];

/// Validate the evidence emitted by the real M18 guest browser acceptance run.
pub(crate) fn validate_serial_log(serial: &str) -> Result<(), String> {
    let mut verified_hosts = [false; REQUIRED_HTTPS_HOSTS.len()];
    let mut tls_evidence_lines = [None; REQUIRED_HTTPS_HOSTS.len()];
    let mut chrome_before_tls = [false; REQUIRED_HTTPS_HOSTS.len()];
    let mut chrome_presented_hosts = [false; REQUIRED_HTTPS_HOSTS.len()];
    let mut rendered_hosts = [false; REQUIRED_HTTPS_HOSTS.len()];
    let mut rendered_before_tls = [false; REQUIRED_HTTPS_HOSTS.len()];
    let mut rendered_before_chrome = [false; REQUIRED_HTTPS_HOSTS.len()];
    let mut last_page_line = None;
    let mut first_page_line = None;
    let mut input_navigation_line = None;
    let mut summary_line = None;

    for (line_number, line) in serial.lines().enumerate() {
        if line.starts_with("Nagi M18 ") && line.contains(" FAIL") {
            return Err(format!(
                "guest reported failure on line {}: {line}",
                line_number + 1
            ));
        }

        if line == INPUT_NAVIGATION_LINE && input_navigation_line.replace(line_number).is_some() {
            return Err("duplicate address-bar input navigation evidence".to_owned());
        }

        if let Some(evidence) = line.strip_prefix(TLS_PASS_PREFIX) {
            let tokens: Vec<&str> = evidence.split_whitespace().collect();
            let host = token_value(&tokens, "host=")
                .ok_or_else(|| format!("TLS evidence on line {} has no host", line_number + 1))?;
            let host_index = expected_host_index(host, line_number)?;
            if tls_evidence_lines[host_index]
                .replace(line_number)
                .is_some()
            {
                return Err(format!("duplicate TLS evidence for {host}"));
            }
            if !tokens.contains(&"chain=verified") || !tokens.contains(&"hostname=verified") {
                return Err(format!(
                    "certificate chain and hostname were not both verified for {host}"
                ));
            }
            verified_hosts[host_index] = true;
        }

        if let Some(evidence) = line.strip_prefix(CHROME_PRESENTED_PREFIX) {
            let tokens: Vec<&str> = evidence.split_whitespace().collect();
            let host = token_value(&tokens, "host=").ok_or_else(|| {
                format!("chrome evidence on line {} has no host", line_number + 1)
            })?;
            let host_index = expected_host_index(host, line_number)?;
            if !verified_hosts[host_index] {
                chrome_before_tls[host_index] = true;
            }
            if chrome_presented_hosts[host_index] {
                return Err(format!("duplicate browser chrome evidence for {host}"));
            }
            chrome_presented_hosts[host_index] = true;
        }

        if let Some(evidence) = line.strip_prefix(PAGE_RENDERED_PREFIX) {
            let tokens: Vec<&str> = evidence.split_whitespace().collect();
            let host = token_value(&tokens, "host=")
                .ok_or_else(|| format!("page evidence on line {} has no host", line_number + 1))?;
            let host_index = expected_host_index(host, line_number)?;
            if rendered_hosts[host_index] {
                return Err(format!("duplicate rendered page evidence for {host}"));
            }
            if !verified_hosts[host_index] {
                rendered_before_tls[host_index] = true;
            }
            if !chrome_presented_hosts[host_index] {
                rendered_before_chrome[host_index] = true;
            }

            let checksum = token_value(&tokens, "frame_checksum=")
                .and_then(|value| value.strip_prefix("0x"))
                .and_then(|value| u32::from_str_radix(value, 16).ok())
                .ok_or_else(|| format!("invalid frame checksum for {host}"))?;
            if checksum == 0 {
                return Err(format!("zero frame checksum for {host}"));
            }
            let ink = token_value(&tokens, "ink_pixels=")
                .and_then(|value| value.parse::<u32>().ok())
                .ok_or_else(|| format!("missing ink_pixels for {host}"))?;
            if ink < MIN_INK_PIXELS {
                return Err(format!(
                    "{host} rendered only {ink} non-background pixels; expected at least {MIN_INK_PIXELS} (blank page?)"
                ));
            }

            rendered_hosts[host_index] = true;
            if host_index == 0 {
                first_page_line = Some(line_number);
            }
            last_page_line = Some(line_number);
        }

        if line == SUMMARY_LINE {
            if summary_line.is_some() {
                return Err("duplicate M18 acceptance summary".to_owned());
            }
            summary_line = Some(line_number);
        }
    }

    let input_line = input_navigation_line
        .ok_or_else(|| "missing address-bar input navigation evidence".to_owned())?;
    if tls_evidence_lines[0].is_some_and(|tls_line| input_line <= tls_line) {
        return Err(
            "address-bar input navigation evidence appeared before TLS verification".to_owned(),
        );
    }
    if first_page_line.is_some_and(|page_line| input_line >= page_line) {
        return Err(
            "address-bar input navigation evidence appeared after example.com rendered".to_owned(),
        );
    }

    for (index, host) in REQUIRED_HTTPS_HOSTS.iter().enumerate() {
        if !verified_hosts[index] {
            return Err(format!(
                "missing successful certificate-chain and hostname validation evidence for {host}"
            ));
        }
        if chrome_before_tls[index] {
            return Err(format!(
                "browser chrome presented before HTTPS validation for {host}"
            ));
        }
        if rendered_before_tls[index] {
            return Err(format!(
                "page rendered before HTTPS validation evidence for {host}"
            ));
        }
        if !chrome_presented_hosts[index] {
            return Err(format!(
                "missing browser chrome presentation evidence for {host}"
            ));
        }
        if rendered_before_chrome[index] {
            return Err(format!(
                "page rendered before browser chrome presentation for {host}"
            ));
        }
        if !rendered_hosts[index] {
            return Err(format!("missing rendered HTTPS page evidence for {host}"));
        }
    }

    let (last_page, summary) = match (last_page_line, summary_line) {
        (Some(last_page), Some(summary)) if summary > last_page => (last_page, summary),
        (_, None) => return Err(format!("missing {SUMMARY_LINE}")),
        _ => {
            return Err("M18 acceptance summary appeared before the page evidence".to_owned());
        }
    };
    validate_clipboard_evidence(serial, last_page, summary)
}

fn validate_clipboard_evidence(
    serial: &str,
    last_page: usize,
    summary: usize,
) -> Result<(), String> {
    let mut previous = last_page;
    for expected in std::iter::once(CONTENT_NAVIGATION_LINE)
        .chain(CLIPBOARD_LINES.iter().copied())
        .chain(IME_LINES.iter().copied())
        .chain(UPLOAD_LINES.iter().copied())
    {
        let mut found = None;
        for (line_number, line) in serial.lines().enumerate() {
            if line == expected && found.replace(line_number).is_some() {
                return Err(format!("duplicate clipboard evidence `{expected}`"));
            }
        }
        let line_number =
            found.ok_or_else(|| format!("missing clipboard or IME evidence `{expected}`"))?;
        if line_number <= previous || line_number >= summary {
            return Err(format!(
                "clipboard or IME evidence `{expected}` appeared out of order on line {}",
                line_number + 1
            ));
        }
        previous = line_number;
    }
    // A trusted paste must never be refused; only the deliberate
    // ungestured probe may be denied.
    if let Some((line_number, line)) = serial.lines().enumerate().find(|(_, line)| {
        line.starts_with("Nagi M18 clipboard ")
            && line.contains(" DENIED")
            && *line != CLIPBOARD_UNGESTURED_DENIAL
    }) {
        return Err(format!(
            "clipboard service denied a user operation on line {}: {line}",
            line_number + 1
        ));
    }
    Ok(())
}

fn expected_host_index(host: &str, line_number: usize) -> Result<usize, String> {
    REQUIRED_HTTPS_HOSTS
        .iter()
        .position(|required| *required == host)
        .ok_or_else(|| format!("unexpected HTTPS host {host} on line {}", line_number + 1))
}

fn token_value<'a>(tokens: &'a [&str], prefix: &str) -> Option<&'a str> {
    tokens
        .iter()
        .find_map(|token| token.strip_prefix(prefix))
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_serial() -> String {
        let mut lines = vec!["Nagi Kernel started".to_owned()];
        for (index, host) in REQUIRED_HTTPS_HOSTS.iter().enumerate() {
            lines.push(format!(
                "{TLS_PASS_PREFIX}host={host} chain=verified hostname=verified"
            ));
            if index == 0 {
                lines.push(INPUT_NAVIGATION_LINE.to_owned());
            }
            lines.push(format!("{CHROME_PRESENTED_PREFIX}host={host}"));
            lines.push(format!(
                "{PAGE_RENDERED_PREFIX}host={host} frame_checksum=0x{:08x} ink_pixels=4000",
                index + 1
            ));
        }
        lines.push(CONTENT_NAVIGATION_LINE.to_owned());
        lines.extend(CLIPBOARD_LINES.iter().map(|line| (*line).to_owned()));
        lines.extend(IME_LINES.iter().map(|line| (*line).to_owned()));
        lines.extend(UPLOAD_LINES.iter().map(|line| (*line).to_owned()));
        lines.push(SUMMARY_LINE.to_owned());
        lines.join("\n")
    }

    #[test]
    fn requires_clipboard_copy_paste_and_ungestured_denial_evidence() {
        for line in CLIPBOARD_LINES {
            let missing = valid_serial().replace(&format!("{line}\n"), "");
            assert!(
                validate_serial_log(&missing)
                    .unwrap_err()
                    .contains("missing clipboard or IME evidence"),
                "{line}"
            );
        }
    }

    #[test]
    fn requires_content_navigation_before_clipboard_evidence() {
        let missing = valid_serial().replace(&format!("{CONTENT_NAVIGATION_LINE}\n"), "");
        assert!(validate_serial_log(&missing)
            .unwrap_err()
            .contains("missing clipboard or IME evidence"));
    }

    #[test]
    fn requires_upload_evidence_after_the_ime() {
        for line in UPLOAD_LINES {
            let missing = valid_serial().replace(&format!("{line}\n"), "");
            assert!(validate_serial_log(&missing)
                .unwrap_err()
                .contains("missing clipboard or IME evidence"));
        }
    }

    #[test]
    fn requires_ime_evidence_after_clipboard_evidence() {
        for line in IME_LINES {
            let missing = valid_serial().replace(&format!("{line}\n"), "");
            assert!(validate_serial_log(&missing)
                .unwrap_err()
                .contains("missing clipboard or IME evidence"));
        }
        let early = valid_serial()
            .replace(&format!("{}\n", IME_LINES[0]), "")
            .replace(
                CLIPBOARD_LINES[0],
                &format!("{}\n{}", IME_LINES[0], CLIPBOARD_LINES[0]),
            );
        assert!(validate_serial_log(&early)
            .unwrap_err()
            .contains("out of order"));
    }

    #[test]
    fn rejects_out_of_order_or_denied_clipboard_evidence() {
        let swapped = valid_serial()
            .replace(CLIPBOARD_LINES[1], "SWAP")
            .replace(CLIPBOARD_LINES[2], CLIPBOARD_LINES[1])
            .replace("SWAP", CLIPBOARD_LINES[2]);
        assert!(validate_serial_log(&swapped)
            .unwrap_err()
            .contains("out of order"));

        let denied = valid_serial().replace(
            CLIPBOARD_LINES[4],
            &format!(
                "Nagi M18 clipboard read DENIED reason=no-user-gesture\n{}",
                CLIPBOARD_LINES[4]
            ),
        );
        assert!(validate_serial_log(&denied)
            .unwrap_err()
            .contains("denied a user operation"));
    }

    #[test]
    fn accepts_three_chain_and_hostname_verified_pages_with_nonzero_frames() {
        assert_eq!(validate_serial_log(&valid_serial()), Ok(()));
    }

    #[test]
    fn rejects_blank_pages_without_ink() {
        let blank = valid_serial().replacen("ink_pixels=4000", "ink_pixels=0", 1);
        assert!(validate_serial_log(&blank)
            .unwrap_err()
            .contains("non-background pixels"));
        let missing = valid_serial().replacen(" ink_pixels=4000", "", 1);
        assert!(validate_serial_log(&missing)
            .unwrap_err()
            .contains("missing ink_pixels"));
    }

    #[test]
    fn rejects_missing_page_evidence() {
        let serial = valid_serial().replace(
            "Nagi M18 HTTPS page RENDERED host=example.org frame_checksum=0x00000002 ink_pixels=4000\n",
            "",
        );
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("rendered HTTPS page evidence for example.org"));
    }

    #[test]
    fn rejects_missing_browser_chrome_presentation() {
        let serial =
            valid_serial().replace("Nagi M18 browser chrome PRESENTED host=example.org\n", "");
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("missing browser chrome presentation evidence for example.org"));
    }

    #[test]
    fn rejects_missing_address_bar_input_navigation_evidence() {
        let serial = valid_serial().replace(&format!("{INPUT_NAVIGATION_LINE}\n"), "");
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("address-bar input navigation evidence"));
    }

    #[test]
    fn rejects_duplicate_address_bar_input_navigation_evidence() {
        let serial = valid_serial().replace(
            &format!("{INPUT_NAVIGATION_LINE}\n"),
            &format!("{INPUT_NAVIGATION_LINE}\n{INPUT_NAVIGATION_LINE}\n"),
        );
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("duplicate address-bar input navigation evidence"));
    }

    #[test]
    fn rejects_duplicate_tls_evidence() {
        let tls = format!("{TLS_PASS_PREFIX}host=example.com chain=verified hostname=verified\n");
        let serial = valid_serial().replace(&tls, &format!("{tls}{tls}"));
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("duplicate TLS evidence for example.com"));
    }

    #[test]
    fn rejects_missing_tls_evidence() {
        let serial = valid_serial().replace(
            "Nagi M18 HTTPS TLS PASS host=example.org chain=verified hostname=verified\n",
            "",
        );
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("certificate-chain and hostname validation evidence for example.org"));
    }

    #[test]
    fn rejects_unverified_chain_or_hostname() {
        let serial = valid_serial().replace("hostname=verified", "hostname=unchecked");
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("were not both verified"));
    }

    #[test]
    fn rejects_page_rendered_before_tls_validation() {
        let tls = format!("{TLS_PASS_PREFIX}host=example.org chain=verified hostname=verified");
        let chrome = format!("{CHROME_PRESENTED_PREFIX}host=example.org");
        let page = format!(
            "{PAGE_RENDERED_PREFIX}host=example.org frame_checksum=0x00000002 ink_pixels=4000"
        );
        let serial = valid_serial().replace(
            &format!("{tls}\n{chrome}\n{page}"),
            &format!("{page}\n{tls}\n{chrome}"),
        );
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("rendered before HTTPS validation"));
    }

    #[test]
    fn rejects_browser_chrome_before_tls_validation() {
        let tls = format!("{TLS_PASS_PREFIX}host=example.org chain=verified hostname=verified");
        let chrome = format!("{CHROME_PRESENTED_PREFIX}host=example.org");
        let serial =
            valid_serial().replace(&format!("{tls}\n{chrome}"), &format!("{chrome}\n{tls}"));
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("browser chrome presented before HTTPS validation for example.org"));
    }

    #[test]
    fn rejects_duplicate_rendered_page_events() {
        let page = format!(
            "{PAGE_RENDERED_PREFIX}host=example.org frame_checksum=0x00000002 ink_pixels=4000"
        );
        let serial = valid_serial().replace(&format!("{page}\n"), &format!("{page}\n{page}\n"));
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("duplicate rendered page evidence for example.org"));
    }

    #[test]
    fn rejects_zero_frame_checksum() {
        let serial =
            valid_serial().replace("frame_checksum=0x00000001", "frame_checksum=0x00000000");
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("zero frame checksum"));
    }

    #[test]
    fn rejects_malformed_checksums_and_unexpected_hosts() {
        let malformed =
            valid_serial().replace("frame_checksum=0x00000001", "frame_checksum=0xnot-hex");
        assert!(validate_serial_log(&malformed)
            .unwrap_err()
            .contains("invalid frame checksum"));

        let unexpected = valid_serial().replace("host=example.net", "host=example.invalid");
        assert!(validate_serial_log(&unexpected)
            .unwrap_err()
            .contains("unexpected HTTPS host"));
    }

    #[test]
    fn rejects_guest_failure_even_if_later_pages_pass() {
        let serial = format!(
            "Nagi M18 TLS validation FAIL host=example.org\n{}",
            valid_serial()
        );
        assert!(validate_serial_log(&serial)
            .unwrap_err()
            .contains("guest reported failure"));
    }

    #[test]
    fn requires_summary_after_all_page_evidence_and_only_once() {
        let pages = valid_serial().replace(SUMMARY_LINE, "");
        let early_summary = format!("{SUMMARY_LINE}\n{pages}");
        let last_page = early_summary.rfind(PAGE_RENDERED_PREFIX).unwrap();
        let summary = early_summary.find(SUMMARY_LINE).unwrap();
        assert!(summary < last_page);
        let error = validate_serial_log(&early_summary).unwrap_err();
        assert!(
            error.contains("appeared before"),
            "unexpected error: {error}"
        );

        let duplicate = format!("{}\n{}\n", valid_serial(), SUMMARY_LINE);
        assert!(validate_serial_log(&duplicate)
            .unwrap_err()
            .contains("duplicate M18 acceptance summary"));
    }
}
