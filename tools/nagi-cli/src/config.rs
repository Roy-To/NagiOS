use std::fs;
use std::path::Path;

use crate::doctor::{ToolchainRequirements, Version};

pub fn validate_project(root: &Path) -> Result<(), String> {
    let manifest = read_required(root, "nagi.toml")?;
    let toolchain = read_required(root, "rust-toolchain.toml")?;
    let source_lock = read_required(root, "third_party/sources.lock")?;

    let manifest_channel = value_in_section(&manifest, "toolchain", "rust_channel")
        .ok_or_else(|| "nagi.toml is missing toolchain.rust_channel".to_owned())?;
    let pinned_channel = quoted_value(&toolchain, "channel")
        .ok_or_else(|| "rust-toolchain.toml is missing a pinned channel".to_owned())?;
    if manifest_channel != pinned_channel {
        return Err(format!(
            "nagi.toml rust_channel `{manifest_channel}` does not match rust-toolchain.toml `{pinned_channel}`"
        ));
    }
    if !source_lock
        .lines()
        .any(|line| line.trim() == "format_version = 1")
    {
        return Err("third_party/sources.lock must declare format_version = 1".to_owned());
    }
    for key in [
        "llvm_min_version",
        "lld_min_version",
        "qemu_min_version",
        "ovmf_pairs",
        "cmake_min_version",
        "meson_min_version",
        "ninja_min_version",
        "python_min_version",
    ] {
        if value_in_section(&manifest, "toolchain", key).is_none() {
            return Err(format!("nagi.toml is missing toolchain.{key}"));
        }
    }
    for key in ["cache", "artifacts", "logs"] {
        if value_in_section(&manifest, "paths", key).is_none() {
            return Err(format!("nagi.toml is missing paths.{key}"));
        }
    }
    Ok(())
}

pub fn load_toolchain_requirements(root: &Path) -> Result<ToolchainRequirements, String> {
    let manifest = read_required(root, "nagi.toml")?;
    Ok(ToolchainRequirements {
        llvm_min_version: version_in_section(&manifest, "llvm_min_version")?,
        lld_min_version: version_in_section(&manifest, "lld_min_version")?,
        qemu_min_version: version_in_section(&manifest, "qemu_min_version")?,
        cmake_min_version: version_in_section(&manifest, "cmake_min_version")?,
        meson_min_version: version_in_section(&manifest, "meson_min_version")?,
        ninja_min_version: version_in_section(&manifest, "ninja_min_version")?,
        python_min_version: version_in_section(&manifest, "python_min_version")?,
        ovmf_pairs: ovmf_pairs_in_manifest(&manifest)?,
    })
}

fn read_required(root: &Path, relative: &str) -> Result<String, String> {
    fs::read_to_string(root.join(relative))
        .map_err(|error| format!("cannot read {relative}: {error}"))
}

fn value_in_section(contents: &str, section: &str, key: &str) -> Option<String> {
    let mut current_section = "";
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed.trim_matches(['[', ']']);
            continue;
        }
        if current_section != section {
            continue;
        }
        let Some((candidate, value)) = trimmed.split_once('=') else {
            continue;
        };
        if candidate.trim() == key {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

fn quoted_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.trim().split_once('=')?;
        (candidate.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
    })
}

fn version_in_section(contents: &str, key: &str) -> Result<Version, String> {
    let value = value_in_section(contents, "toolchain", key)
        .ok_or_else(|| format!("nagi.toml is missing toolchain.{key}"))?;
    let mut parts = value.split('.');
    let major = parts
        .next()
        .ok_or_else(|| format!("toolchain.{key} is not a version"))?
        .parse()
        .map_err(|_| format!("toolchain.{key} has an invalid version: {value}"))?;
    let minor = parts
        .next()
        .unwrap_or("0")
        .parse()
        .map_err(|_| format!("toolchain.{key} has an invalid version: {value}"))?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .parse()
        .map_err(|_| format!("toolchain.{key} has an invalid version: {value}"))?;
    if parts.next().is_some() {
        return Err(format!("toolchain.{key} has an invalid version: {value}"));
    }
    Ok((major, minor, patch))
}

fn ovmf_pairs_in_manifest(contents: &str) -> Result<Vec<(String, String)>, String> {
    let value = value_in_section(contents, "toolchain", "ovmf_pairs")
        .ok_or_else(|| "nagi.toml is missing toolchain.ovmf_pairs".to_owned())?;
    let pairs: Vec<_> = value
        .split(',')
        .filter_map(|pair| pair.split_once(':'))
        .map(|(code, vars)| (code.trim().to_owned(), vars.trim().to_owned()))
        .filter(|(code, vars)| !code.is_empty() && !vars.is_empty())
        .collect();
    if pairs.is_empty() {
        return Err("toolchain.ovmf_pairs must contain CODE:VARS entries".to_owned());
    }
    Ok(pairs)
}
