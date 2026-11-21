#!/usr/bin/env python3
"""Verify the isolated Linux test lock against the root lock and Cargo graph."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
import unittest
from pathlib import Path
from typing import Any

Identity = tuple[str, str, str | None, str | None]
PackageRows = list[dict[str, Any]]
VERSION_RE = re.compile(r"\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?\Z")
TREE_LINE_RE = re.compile(r"(\d+)(\S+) v([^\s(]+)(?: \((.*)\))?\Z")
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"


class LockGraphError(ValueError):
    pass


def package_identity(package: dict[str, Any]) -> Identity:
    return (
        package["name"],
        package["version"],
        package.get("source"),
        package.get("checksum"),
    )


def lock_index(lock: dict[str, Any], label: str) -> tuple[dict[Identity, dict[str, Any]], dict[tuple[str, str], list[Identity]]]:
    packages: dict[Identity, dict[str, Any]] = {}
    by_name_version: dict[tuple[str, str], list[Identity]] = {}
    for package in lock.get("package", []):
        identity = package_identity(package)
        if identity in packages:
            raise LockGraphError(f"{label} contains a duplicate package identity: {identity}")
        packages[identity] = package
        by_name_version.setdefault((identity[0], identity[1]), []).append(identity)
    return packages, by_name_version


def resolve_dependency(
    spec: str,
    packages: dict[Identity, dict[str, Any]],
    by_name_version: dict[tuple[str, str], list[Identity]],
    label: str,
) -> Identity:
    parts = spec.split()
    if not parts:
        raise LockGraphError(f"{label} has an empty dependency entry")
    name = parts[0]
    version = parts[1] if len(parts) > 1 and VERSION_RE.fullmatch(parts[1]) else None
    remainder = parts[2:] if version is not None else parts[1:]
    source = None
    if remainder:
        source_text = " ".join(remainder)
        if source_text.startswith("(") and source_text.endswith(")"):
            source = source_text[1:-1]
        else:
            raise LockGraphError(f"cannot parse dependency source in {spec!r}")

    if version is None:
        matches = [identity for identity in packages if identity[0] == name]
    else:
        matches = list(by_name_version.get((name, version), []))
    if source is not None:
        matches = [identity for identity in matches if identity[2] == source]
    if len(matches) != 1:
        raise LockGraphError(
            f"{label} dependency {spec!r} resolves to {len(matches)} package records"
        )
    return matches[0]


def lock_graph(
    lock: dict[str, Any], label: str
) -> tuple[dict[Identity, dict[str, Any]], dict[Identity, set[Identity]]]:
    packages, by_name_version = lock_index(lock, label)
    edges: dict[Identity, set[Identity]] = {}
    for identity, package in packages.items():
        edges[identity] = {
            resolve_dependency(spec, packages, by_name_version, f"{label} {identity[0]} {identity[1]}")
            for spec in package.get("dependencies", [])
        }
    return packages, edges


def find_root(packages: dict[Identity, dict[str, Any]], name: str) -> Identity:
    matches = [identity for identity in packages if identity[0] == name]
    if len(matches) != 1:
        raise LockGraphError(f"expected one {name!r} root package, found {len(matches)}")
    return matches[0]


def reachable(edges: dict[Identity, set[Identity]], root: Identity) -> set[Identity]:
    if root not in edges:
        raise LockGraphError(f"root package is absent from dependency graph: {root}")
    found = {root}
    pending = [root]
    while pending:
        for child in edges[pending.pop()]:
            if child not in found:
                found.add(child)
                pending.append(child)
    return found


def parse_cargo_tree(
    lines: list[str],
    by_name_version: dict[tuple[str, str], list[Identity]],
    expected_root: Identity,
    expected_paths: dict[Identity, Path],
) -> tuple[set[Identity], set[tuple[Identity, Identity]]]:
    nodes: set[Identity] = set()
    edges: set[tuple[Identity, Identity]] = set()
    stack: list[Identity] = []
    roots = 0
    for line_number, raw_line in enumerate(lines, start=1):
        line = raw_line.strip()
        if not line:
            continue
        match = TREE_LINE_RE.fullmatch(line)
        if match is None:
            raise LockGraphError(f"cannot parse cargo tree line {line_number}: {raw_line!r}")
        depth, name, version = int(match.group(1)), match.group(2), match.group(3)
        source_display = match.group(4)
        candidates = by_name_version.get((name, version), [])
        if len(candidates) != 1:
            raise LockGraphError(
                f"cargo tree package {name} {version} maps to {len(candidates)} lock records"
            )
        child = candidates[0]
        lock_source = child[2]
        if lock_source is None:
            expected_path = expected_paths.get(child)
            if expected_path is None or source_display is None:
                raise LockGraphError(f"cargo tree omitted a verifiable path source for {child}")
            if Path(source_display).resolve() != expected_path:
                raise LockGraphError(
                    f"cargo tree path source differs from manifest for {child}: {source_display!r}"
                )
        elif lock_source.startswith("registry+"):
            if source_display not in (None, "proc-macro"):
                raise LockGraphError(
                    f"cargo tree source marker conflicts with registry lock source for {child}: "
                    f"{source_display!r}"
                )
        else:
            raise LockGraphError(
                f"cargo tree source form is not verified for non-registry package {child}: {lock_source}"
            )
        if depth == 0:
            roots += 1
            if child != expected_root:
                raise LockGraphError(f"cargo tree root differs from Job lock root: {child}")
            stack = [child]
        else:
            if depth > len(stack):
                raise LockGraphError(f"invalid cargo tree depth on line {line_number}")
            parent = stack[depth - 1]
            edges.add((parent, child))
            stack = stack[:depth] + [child]
        nodes.add(child)
    if roots != 1:
        raise LockGraphError(f"expected one cargo tree root, found {roots}")
    return nodes, edges


def validate_projection(
    root_lock: dict[str, Any],
    projected_lock: dict[str, Any],
    tree_lines: list[str],
    expected_paths: dict[Identity, Path],
    root_name: str = "nagi-job-scheduler",
) -> tuple[int, int, str]:
    root_packages, root_edges = lock_graph(root_lock, "root Cargo.lock")
    projected_packages, projected_edges = lock_graph(projected_lock, "projected Cargo.lock")
    root_id = find_root(root_packages, root_name)
    projected_id = find_root(projected_packages, root_name)
    if projected_id != root_id:
        raise LockGraphError(f"isolated root package does not match root Cargo.lock: {projected_id} != {root_id}")

    projected_closure = reachable(projected_edges, projected_id)
    if projected_closure != set(projected_packages):
        extras = set(projected_packages) - projected_closure
        raise LockGraphError(f"projected Cargo.lock contains unreachable extra package records: {sorted(extras)}")

    root_closure = reachable(root_edges, root_id)
    if not projected_closure <= root_closure:
        extras = projected_closure - root_closure
        raise LockGraphError(f"projected dependency closure is absent from root lock closure: {sorted(extras)}")

    for package_id in projected_closure:
        if package_id not in root_packages:
            raise LockGraphError(f"projected package source/checksum/version absent from root lock: {package_id}")
        extra_edges = projected_edges[package_id] - root_edges[package_id]
        if extra_edges:
            raise LockGraphError(
                f"projected dependency edges are absent from root lock for {package_id}: {sorted(extra_edges)}"
            )

    tree_nodes, tree_edges = parse_cargo_tree(
        tree_lines,
        lock_index(projected_lock, "projected Cargo.lock")[1],
        projected_id,
        expected_paths,
    )
    if tree_nodes != projected_closure:
        missing = projected_closure - tree_nodes
        extra = tree_nodes - projected_closure
        raise LockGraphError(f"cargo tree node closure differs from lock (missing={sorted(missing)}, extra={sorted(extra)})")
    projected_edge_pairs = {
        (parent, child)
        for parent, children in projected_edges.items()
        for child in children
    }
    if tree_edges != projected_edge_pairs:
        missing = projected_edge_pairs - tree_edges
        extra = tree_edges - projected_edge_pairs
        raise LockGraphError(f"cargo tree edges differ from lock (missing={sorted(missing)}, extra={sorted(extra)})")

    serde_json_versions = sorted(
        identity[1] for identity in projected_closure if identity[0] == "serde_json"
    )
    if len(serde_json_versions) != 1:
        raise LockGraphError(f"expected one serde_json package in Job closure, found {serde_json_versions}")
    return len(projected_closure), len(projected_edge_pairs), serde_json_versions[0]


def load_lock(path: str) -> dict[str, Any]:
    with Path(path).open("rb") as stream:
        return tomllib.load(stream)


def path_sources(projected_lock: dict[str, Any], projected_manifest_path: str) -> dict[Identity, Path]:
    manifest_path = Path(projected_manifest_path).resolve()
    manifest = tomllib.loads(manifest_path.read_text())
    packages, _ = lock_index(projected_lock, "projected Cargo.lock")
    root_id = find_root(packages, manifest["package"]["name"])
    sources = {root_id: manifest_path.parent}
    for patch_table in manifest.get("patch", {}).values():
        for package_name, patch in patch_table.items():
            if not isinstance(patch, dict) or "path" not in patch:
                continue
            matches = [
                identity
                for identity in packages
                if identity[0] == package_name and identity[2] is None
            ]
            if len(matches) != 1:
                raise LockGraphError(
                    f"expected one path-patched lock package for {package_name}, found {len(matches)}"
                )
            patch_path = Path(patch["path"])
            if not patch_path.is_absolute():
                patch_path = manifest_path.parent / patch_path
            sources[matches[0]] = patch_path.resolve()
    return sources


def check_projection(
    root_path: str, projected_path: str, tree_path: str, projected_manifest_path: str
) -> None:
    root_lock = load_lock(root_path)
    projected_lock = load_lock(projected_path)
    tree_lines = Path(tree_path).read_text().splitlines()
    expected_paths = path_sources(projected_lock, projected_manifest_path)
    nodes, edges, serde_json_version = validate_projection(
        root_lock, projected_lock, tree_lines, expected_paths
    )
    print(
        f"verified complete Job lock projection: {nodes} reachable packages, {edges} dependency edges; "
        f"serde_json {serde_json_version}; every package source/checksum and edge matches root Cargo.lock"
    )


def package(name: str, version: str, dependencies: list[str] | None = None,
            source: str | None = None, checksum: str | None = None) -> dict[str, Any]:
    result: dict[str, Any] = {"name": name, "version": version, "dependencies": dependencies or []}
    if source is not None:
        result["source"] = source
    if checksum is not None:
        result["checksum"] = checksum
    return result


def sample_locks() -> tuple[dict[str, Any], dict[str, Any], list[str]]:
    app = package("nagi-job-scheduler", "0.2.0", ["serde_json 1.0.151"])
    serde_root = package(
        "serde_json", "1.0.151", ["helper 0.1.0", "root-feature-extra 0.1.0"], CRATES_IO, "a" * 64
    )
    helper = package("helper", "0.1.0", [], CRATES_IO, "b" * 64)
    root_extra = package("root-feature-extra", "0.1.0", [], CRATES_IO, "c" * 64)
    root = {"package": [app, serde_root, helper, root_extra]}
    projected = {
        "package": [
            {**item, "dependencies": list(item["dependencies"])}
            for item in (app, serde_root, helper)
        ]
    }
    projected["package"][1]["dependencies"] = ["helper 0.1.0"]
    tree = [
        "0nagi-job-scheduler v0.2.0 (/tmp/job)",
        "1serde_json v1.0.151",
        "2helper v0.1.0",
    ]
    return root, projected, tree


def sample_paths() -> dict[Identity, Path]:
    return {
        ("nagi-job-scheduler", "0.2.0", None, None): Path("/tmp/job").resolve(),
    }


class ProjectionNegativeTests(unittest.TestCase):
    def test_accepts_feature_union_superset_in_root_lock(self) -> None:
        root, projected, tree = sample_locks()
        self.assertEqual(
            validate_projection(root, projected, tree, sample_paths()), (3, 2, "1.0.151")
        )

    def test_rejects_missing_dependency_edge(self) -> None:
        root, projected, tree = sample_locks()
        projected["package"][1]["dependencies"] = []
        with self.assertRaisesRegex(LockGraphError, "extra package records|differs from lock"):
            validate_projection(root, projected, tree, sample_paths())

    def test_rejects_missing_dependency_package(self) -> None:
        root, projected, tree = sample_locks()
        projected["package"] = projected["package"][:-1]
        with self.assertRaisesRegex(LockGraphError, "resolves to 0 package records"):
            validate_projection(root, projected, tree, sample_paths())

    def test_rejects_replaced_package_source_or_checksum(self) -> None:
        for field, replacement in (
            ("source", "git+https://example.invalid/serde_json#deadbeef"),
            ("checksum", "d" * 64),
        ):
            with self.subTest(field=field):
                root, projected, tree = sample_locks()
                projected["package"][1][field] = replacement
                with self.assertRaisesRegex(LockGraphError, "absent from root lock"):
                    validate_projection(root, projected, tree, sample_paths())

    def test_rejects_unreachable_extra_package_record(self) -> None:
        root, projected, tree = sample_locks()
        projected["package"].append(package("unused", "0.1.0", [], CRATES_IO, "e" * 64))
        with self.assertRaisesRegex(LockGraphError, "unreachable extra package records"):
            validate_projection(root, projected, tree, sample_paths())

    def test_rejects_extra_dependency_edge(self) -> None:
        root, projected, tree = sample_locks()
        projected["package"][1]["dependencies"] = ["helper 0.1.0", "root-feature-extra 0.1.0"]
        projected["package"].append(root["package"][3].copy())
        with self.assertRaisesRegex(LockGraphError, "differs from lock"):
            validate_projection(root, projected, tree, sample_paths())

    def test_rejects_direct_edge_absent_from_root_lock(self) -> None:
        root, projected, tree = sample_locks()
        projected["package"][0]["dependencies"].append("helper 0.1.0")
        with self.assertRaisesRegex(LockGraphError, "dependency edges are absent from root lock"):
            validate_projection(root, projected, tree, sample_paths())

    def test_rejects_missing_cargo_tree_edge(self) -> None:
        root, projected, _ = sample_locks()
        root["package"][0]["dependencies"].append("helper 0.1.0")
        projected["package"][0]["dependencies"].append("helper 0.1.0")
        tree = [
            "0nagi-job-scheduler v0.2.0 (/tmp/job)",
            "1serde_json v1.0.151",
            "1helper v0.1.0",
        ]
        with self.assertRaisesRegex(LockGraphError, "cargo tree edges differ from lock"):
            validate_projection(root, projected, tree, sample_paths())

    def test_rejects_extra_cargo_tree_edge(self) -> None:
        root, projected, _ = sample_locks()
        tree = [
            "0nagi-job-scheduler v0.2.0 (/tmp/job)",
            "1serde_json v1.0.151",
            "2helper v0.1.0",
            "1helper v0.1.0",
        ]
        with self.assertRaisesRegex(LockGraphError, "cargo tree edges differ from lock"):
            validate_projection(root, projected, tree, sample_paths())

    def test_rejects_cargo_tree_path_source_mismatch(self) -> None:
        root, projected, tree = sample_locks()
        tree[0] = "0nagi-job-scheduler v0.2.0 (/tmp/other-job)"
        with self.assertRaisesRegex(LockGraphError, "path source differs from manifest"):
            validate_projection(root, projected, tree, sample_paths())

    def test_rejects_same_name_version_from_multiple_sources(self) -> None:
        root, projected, tree = sample_locks()
        alternate_source = "git+https://example.invalid/serde_json#deadbeef"
        alternate = package("serde_json", "1.0.151", [], alternate_source)
        root["package"].append(alternate.copy())
        projected["package"].append(alternate.copy())
        registry_source = root["package"][1]["source"]
        registry_dependency = f"serde_json 1.0.151 ({registry_source})"
        git_dependency = f"serde_json 1.0.151 ({alternate_source})"
        root["package"][0]["dependencies"] = [registry_dependency, git_dependency]
        projected["package"][0]["dependencies"] = [registry_dependency, git_dependency]
        with self.assertRaisesRegex(LockGraphError, "maps to 2 lock records"):
            validate_projection(root, projected, tree, sample_paths())


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--root-lock")
    parser.add_argument("--projected-lock")
    parser.add_argument("--cargo-tree")
    parser.add_argument("--projected-manifest")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(ProjectionNegativeTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    if not args.root_lock or not args.projected_lock or not args.cargo_tree or not args.projected_manifest:
        parser.error(
            "--root-lock, --projected-lock, --cargo-tree and --projected-manifest "
            "are required without --self-test"
        )
    try:
        check_projection(
            args.root_lock, args.projected_lock, args.cargo_tree, args.projected_manifest
        )
    except (OSError, tomllib.TOMLDecodeError, LockGraphError) as error:
        print(f"root lock projection verification failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
