use std::fs;
use std::path::Path;

fn lock_string_field(section: &str, field: &str) -> Option<String> {
    section.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() != field {
            return None;
        }
        value
            .trim()
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .map(str::to_owned)
    })
}

#[test]
fn third_party_notices_cover_every_pinned_component_and_declared_license() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lock =
        fs::read_to_string(root.join("third_party/sources.lock")).expect("third-party source lock");
    let notices =
        fs::read_to_string(root.join("THIRD_PARTY_NOTICES.md")).expect("third-party notices");
    let notices = notices.to_ascii_lowercase();
    let mut components_checked = 0usize;

    for entry in lock.split("[sources.").skip(1) {
        let (_, body) = entry.split_once(']').expect("source entry section header");
        let body = body.split("\n[").next().unwrap_or(body);
        let component = lock_string_field(body, "component").expect("component field");
        assert!(
            notices.contains(&component.to_ascii_lowercase()),
            "THIRD_PARTY_NOTICES.md omits pinned component `{component}`"
        );

        let pins: Vec<_> = ["version", "revision", "toolchain"]
            .into_iter()
            .filter_map(|field| lock_string_field(body, field))
            .collect();
        assert!(!pins.is_empty(), "{component} is missing a visible pin");
        for pin in pins {
            assert!(
                notices.contains(&pin.to_ascii_lowercase()),
                "THIRD_PARTY_NOTICES.md omits pin `{pin}` for `{component}`"
            );
        }

        if component == "rust-std" {
            assert!(
                lock_string_field(body, "license").is_none(),
                "rust-std license metadata should remain explicitly unresolved"
            );
            assert!(
                notices.contains("upstream redistribution metadata is not complete in the lock")
            );
        } else {
            let license = lock_string_field(body, "license")
                .unwrap_or_else(|| panic!("{component} is missing a lockfile license"));
            assert!(
                notices.contains(&license.to_ascii_lowercase()),
                "THIRD_PARTY_NOTICES.md omits declared license `{license}` for `{component}`"
            );
        }
        components_checked += 1;
    }

    assert!(
        components_checked >= 18,
        "expected the current pinned source set"
    );
}
