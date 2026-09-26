use nagi_localization::{validate_bundled_catalogs, CatalogSet};

fn main() {
    if let Err(issues) = validate_bundled_catalogs() {
        for issue in issues {
            eprintln!("FAIL {issue}");
        }
        std::process::exit(1);
    }

    match CatalogSet::bundled() {
        Ok(catalogs) => {
            let locale_count = catalogs.iter().count();
            let message_count: usize = catalogs.iter().map(|catalog| catalog.message_count()).sum();
            println!(
                "PASS localization catalogs: {locale_count} locales, {message_count} messages"
            );
        }
        Err(error) => {
            eprintln!("FAIL localization catalogs: {error}");
            std::process::exit(1);
        }
    }
}
