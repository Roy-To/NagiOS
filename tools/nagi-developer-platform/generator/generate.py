#!/usr/bin/env python3
"""NDP-03 Level 0 template generator pilot (host-only, Python 3.9 stdlib).

Contents come from the NDP-02 contract preview (`contract/contract.py`); this
module only decides *where* and *whether* to write, following the contract's
`writePolicy`: explicit new directory only, no overwrite/merge, no symlinks in
the destination or its ancestors, exclusive creation, and on failure removal of
only the files/directories this invocation created (verified by the identity
recorded from the creating handle; replaced objects are kept and reported).

It never builds, fetches, installs, executes or validates the generated output.
"""
import argparse
import errno
import importlib.util
import json
import os
import secrets
import stat
import sys
from pathlib import Path, PurePosixPath

GENERATOR_DIR = Path(__file__).resolve().parent


def _load_contract_module():
    # Loaded by path under a private name: `contract/` is intentionally not a
    # package, and its own tests import it as the top-level module `contract`.
    spec = importlib.util.spec_from_file_location(
        "_ndp03_level0_contract", GENERATOR_DIR / "contract" / "contract.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


contract_module = _load_contract_module()
REPO_ROOT = contract_module.REPO_ROOT

# PROVISIONAL generator policy (see README). Destinations inside these
# repo-relative trees are rejected so the generator cannot modify the SDK,
# package tooling or VCS metadata. Replace this data to change the policy.
PROTECTED_TREES = (".git", "sdk", "tools/nagi-pkg")

_HOST_FLAGS = ("O_DIRECTORY", "O_NOFOLLOW")
_DIR_FLAGS = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0)


class GenerationError(Exception):
    """Stable machine-readable `code`; never contains absolute host paths."""

    def __init__(self, code, created=(), removed=(), left_behind=()):
        super().__init__(code)
        self.code = code
        self.created = list(created)
        self.removed = list(removed)
        self.left_behind = list(left_behind)

    def report(self):
        return {"status": "failed", "reason": self.code, "built": False,
                "created": self.created, "removed": self.removed,
                "left_behind": self.left_behind}


def _is_protected(output_dir):
    target = PurePosixPath(output_dir)
    return any(target == PurePosixPath(tree) or PurePosixPath(tree) in target.parents
               for tree in PROTECTED_TREES)


def _open_directory(parent_fd, name, code):
    """Open a real (non-symlink) directory below `parent_fd`."""
    try:
        info = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    except FileNotFoundError:
        raise GenerationError(code + "_missing") from None
    except OSError:
        raise GenerationError(code + "_unavailable") from None
    if stat.S_ISLNK(info.st_mode):
        raise GenerationError("symlink_in_path")
    if not stat.S_ISDIR(info.st_mode):
        raise GenerationError(code + "_not_directory")
    try:
        return os.open(name, _DIR_FLAGS, dir_fd=parent_fd)
    except OSError:
        # Raced into a symlink/non-directory between lstat and open.
        raise GenerationError("symlink_in_path") from None


def _walk_directories(root_fd, parts, code):
    """Return an fd for root/parts..., opening each level without following links."""
    current = os.dup(root_fd)
    try:
        for part in parts:
            child = _open_directory(current, part, code)
            os.close(current)
            current = child
        return current
    except BaseException:
        os.close(current)
        raise


def _check_sdk_present(root_fd, sdk_reference):
    parts = PurePosixPath(sdk_reference["repoPath"]).parts
    try:
        directory = _walk_directories(root_fd, parts, "sdk")
    except GenerationError as error:
        raise GenerationError("sdk_checkout_not_found" if error.code.startswith("sdk_")
                              else error.code) from None
    try:
        info = os.stat("Cargo.toml", dir_fd=directory, follow_symlinks=False)
    except OSError:
        raise GenerationError("sdk_checkout_not_found") from None
    finally:
        os.close(directory)
    if not stat.S_ISREG(info.st_mode):
        raise GenerationError("sdk_checkout_not_found")


def _open_root(dest_root):
    if not all(hasattr(os, flag) for flag in _HOST_FLAGS):
        raise GenerationError("unsupported_host")
    try:
        # The caller-selected root itself may be reached through a symlinked
        # prefix (e.g. /tmp on macOS); everything below it is no-follow.
        return os.open(os.path.realpath(os.fspath(dest_root)), _DIR_FLAGS)
    except OSError:
        raise GenerationError("destination_root_unavailable") from None


def _preflight(inputs, dest_root, source_root, contract):
    """Validate inputs and the destination read-only. Returns (files, root_fd, parent_fd, name)."""
    try:
        files = contract_module.preview(inputs, contract, source_root)
    except (ValueError, KeyError, TypeError, OSError) as error:
        raise GenerationError(str(error) if isinstance(error, ValueError)
                              else "contract_unavailable") from None
    output_dir = inputs["output_dir"]
    if _is_protected(output_dir):
        raise GenerationError("protected_output_directory")
    parts = PurePosixPath(output_dir).parts
    root_fd = _open_root(dest_root)
    try:
        _check_sdk_present(root_fd, contract["sdkReference"])
        parent_fd = _walk_directories(root_fd, parts[:-1], "output_parent")
    except BaseException:
        os.close(root_fd)
        raise
    try:
        try:
            info = os.stat(parts[-1], dir_fd=parent_fd, follow_symlinks=False)
        except FileNotFoundError:
            info = None
        except OSError:
            raise GenerationError("output_unavailable") from None
        if info is not None:
            raise GenerationError("output_symlink" if stat.S_ISLNK(info.st_mode)
                                  else "output_exists")
    except BaseException:
        os.close(parent_fd)
        os.close(root_fd)
        raise
    return files, root_fd, parent_fd, parts[-1]


def plan(inputs, dest_root, source_root=REPO_ROOT, contract=None):
    """Return the planned files after the same checks as `generate`; writes nothing."""
    contract = contract_module.load_contract() if contract is None else contract
    files, root_fd, parent_fd, _ = _preflight(inputs, dest_root, source_root, contract)
    os.close(parent_fd)
    os.close(root_fd)
    return {"status": "planned", "scope": "host_files_only", "built": False,
            "output_dir": inputs["output_dir"],
            "files": [{"path": path, "bytes": len(text.encode("utf-8"))}
                      for path, text in files.items()]}


def _identity(info):
    return (info.st_dev, info.st_ino, stat.S_IFMT(info.st_mode))


def _current_identity(parent_fd, name):
    try:
        return _identity(os.stat(name, dir_fd=parent_fd, follow_symlinks=False))
    except OSError:
        return None


def _restore(quarantine_fd, slot, parent_fd, name, kind):
    """Put a foreign object moved into quarantine back under `name` without clobbering."""
    try:
        if kind == stat.S_IFDIR:
            if _current_identity(parent_fd, name) is not None:
                return False
            os.rename(slot, name, src_dir_fd=quarantine_fd, dst_dir_fd=parent_fd)
        else:
            # link() never replaces an existing name (EEXIST), unlike rename().
            os.link(slot, name, src_dir_fd=quarantine_fd, dst_dir_fd=parent_fd,
                    follow_symlinks=False)
            os.unlink(slot, dir_fd=quarantine_fd)
        return True
    except OSError:
        return False


def _remove_file(entry, quarantine, index):
    """Delete a created file only if the object we delete is provably the one we created.

    POSIX has no unlink-by-handle, so the name is first moved into a private,
    freshly created quarantine directory; only after the moved object's identity
    matches the recorded (dev, inode, type) is it unlinked. A foreign object
    (replaced between our check and the move) is put back without clobbering,
    or left in quarantine and reported. Returns a label for `left_behind` or None.
    """
    label, parent_fd, name, created = entry
    if _current_identity(parent_fd, name) != created:
        return label                                  # replaced/removed by someone else
    quarantine_fd, quarantine_label = quarantine
    slot = str(index)
    try:
        os.rename(name, slot, src_dir_fd=parent_fd, dst_dir_fd=quarantine_fd)
    except OSError:
        return label
    moved = _current_identity(quarantine_fd, slot)
    if moved == created:
        try:
            os.unlink(slot, dir_fd=quarantine_fd)
            return None
        except OSError:
            return quarantine_label + "/" + slot
    if moved is not None and _restore(quarantine_fd, slot, parent_fd, name, moved[2]):
        return label
    return quarantine_label + "/" + slot


def _open_quarantine(parent_fd, name, output_dir):
    """Create an exclusive empty directory next to the output for safe cleanup."""
    for _ in range(8):
        quarantine = ".{}.generator-cleanup-{}".format(name, secrets.token_hex(6))
        try:
            os.mkdir(quarantine, 0o700, dir_fd=parent_fd)
        except FileExistsError:
            continue
        except OSError:
            return None
        try:
            fd = os.open(quarantine, _DIR_FLAGS, dir_fd=parent_fd)
        except OSError:
            return None
        label = str(PurePosixPath(output_dir).parent / quarantine)
        return quarantine, fd, label, _identity(os.fstat(fd))
    return None


def _cleanup(files, directories, top):
    """Remove only what this invocation created. Returns (removed, left_behind).

    `files`/`directories` hold (label, parent_fd, name, identity) recorded from
    the handle at creation time. Anything whose identity no longer matches is
    left in place and reported, never deleted.
    """
    removed, left = [], []
    if not files and not directories:
        return removed, left
    quarantine = None
    if files:
        quarantine = _open_quarantine(top[0], top[1], top[2])
        if quarantine is None:
            # Cannot delete safely: leave everything and report it.
            return removed, ([label for label, _, _, _ in reversed(files)]
                             + [label + "/" for label, _, _, _ in reversed(directories)])
    try:
        for index, entry in enumerate(reversed(files)):
            leftover = _remove_file(entry, (quarantine[1], quarantine[2]), index)
            if leftover is None:
                removed.append(entry[0])
            else:
                left.append(leftover)
        for label, parent_fd, name, created in reversed(directories):
            # rmdir() only removes empty directories, so foreign content is never
            # lost; the identity check keeps a foreign empty replacement as well.
            if _current_identity(parent_fd, name) != created:
                left.append(label + "/")
                continue
            try:
                os.rmdir(name, dir_fd=parent_fd)
                removed.append(label + "/")
            except OSError:
                left.append(label + "/")
    finally:
        if quarantine is not None:
            name, fd, label, created = quarantine
            os.close(fd)
            if _current_identity(top[0], name) != created:
                left.append(label + "/")
            else:
                try:
                    os.rmdir(name, dir_fd=top[0])
                except OSError:
                    left.append(label + "/")
    return removed, left


def generate(inputs, dest_root, source_root=REPO_ROOT, contract=None):
    """Create the Level 0 template in a new directory `dest_root/inputs["output_dir"]`.

    `source_root` supplies the contract's schema; `dest_root` is the selected
    checkout that receives the output and must contain the SDK. Raises
    GenerationError (with created/removed/left_behind) on any failure.
    """
    contract = contract_module.load_contract() if contract is None else contract
    files, root_fd, parent_fd, name = _preflight(inputs, dest_root, source_root, contract)
    output_dir = inputs["output_dir"]
    # (label, parent_fd, name, identity) in creation order; identity comes from
    # the handle we opened, so cleanup can tell our objects from replacements.
    created_files, created_dirs = [], []
    open_fds = [parent_fd, root_fd]
    directory_fds = {}

    def make_directory(parent, child_name, label):
        os.mkdir(child_name, 0o755, dir_fd=parent)   # atomic; fails if anything exists
        child = os.open(child_name, _DIR_FLAGS, dir_fd=parent)
        open_fds.append(child)
        if os.listdir(child):
            # Not the empty directory we just made: someone swapped it in.
            raise GenerationError("output_raced")
        created_dirs.append((label, parent, child_name, _identity(os.fstat(child))))
        return child

    try:
        try:
            out_fd = make_directory(parent_fd, name, output_dir)
        except FileExistsError:
            raise GenerationError("output_exists") from None
        directory_fds[PurePosixPath(".")] = out_fd
        for relative, text in files.items():
            target = PurePosixPath(relative)
            for depth in range(1, len(target.parts)):
                directory = PurePosixPath(*target.parts[:depth])
                if directory in directory_fds:
                    continue
                directory_fds[directory] = make_directory(
                    directory_fds[directory.parent], directory.name,
                    output_dir + "/" + str(directory))
            parent = directory_fds[target.parent]
            descriptor = os.open(target.name,
                                 os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                 0o644, dir_fd=parent)
            with os.fdopen(descriptor, "wb") as handle:
                created_files.append((output_dir + "/" + relative, parent, target.name,
                                      _identity(os.fstat(handle.fileno()))))
                handle.write(text.encode("utf-8"))
    except BaseException as error:
        removed, left = _cleanup(created_files, created_dirs, (parent_fd, name, output_dir))
        if isinstance(error, GenerationError):
            error.created = [entry[0] for entry in created_files + created_dirs]
            error.removed, error.left_behind = removed, left
            raise
        if isinstance(error, OSError):
            code = "write_failed:" + errno.errorcode.get(error.errno, "unknown")
            raise GenerationError(code, [entry[0] for entry in created_files + created_dirs],
                                  removed, left) from None
        raise
    finally:
        for descriptor in open_fds:
            try:
                os.close(descriptor)
            except OSError:
                pass
    return {"status": "created", "scope": "host_files_only", "built": False,
            "validated": False, "output_dir": output_dir,
            "created": [entry[0] for entry in created_files],
            "next": "manual review; manifest validation and build are separate explicit steps"}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("plan", "create"))
    parser.add_argument("--root", default=str(REPO_ROOT),
                        help="checkout that receives the output (must contain sdk/rust)")
    parser.add_argument("--app-id", required=True)
    parser.add_argument("--package-name", required=True)
    parser.add_argument("--name-en", required=True, help="display name en-US")
    parser.add_argument("--name-ja", required=True, help="display name ja-JP")
    parser.add_argument("--output-dir", required=True, help="new repo-relative directory")
    parser.add_argument("--app-version")
    args = parser.parse_args(argv)
    inputs = {"app_id": args.app_id, "package_name": args.package_name,
              "display_name": {"en-US": args.name_en, "ja-JP": args.name_ja},
              "output_dir": args.output_dir}
    if args.app_version is not None:
        inputs["app_version"] = args.app_version
    try:
        result = (plan if args.action == "plan" else generate)(inputs, args.root)
    except GenerationError as error:
        print(json.dumps(error.report(), ensure_ascii=False, sort_keys=True))
        return 1
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
