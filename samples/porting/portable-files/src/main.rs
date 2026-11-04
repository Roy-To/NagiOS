// SPDX-License-Identifier: MPL-2.0

use nagi_portable_files_core::PortableCatalog;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut catalog = PortableCatalog::default();
    catalog.add("notes/porting-plan.md")?;
    println!(
        "contains plan: {}",
        catalog.contains("notes/porting-plan.md")?
    );
    Ok(())
}
