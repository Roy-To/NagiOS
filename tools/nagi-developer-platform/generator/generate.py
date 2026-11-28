#!/usr/bin/env python3
"""NDP-03 Level 0 template generator pilot (host-only, Python 3.9 stdlib).

Contents come from the NDP-02 contract preview (`contract/contract.py`); this
module only decides *where* and *whether* to write, following the contract's
`writePolicy`: explicit new directory only, no overwrite/merge, no symlinks in
the destination or its ancestors, exclusive creation, and never deleting
anything: on failure the objects this invocation created are left in place and
reported, because a concurrent replacement could otherwise make cleanup remove
foreign data.

It never builds, fetches, installs, executes or validates the generated output.
"""
import argparse
import errno
import importlib.util
import json
import os
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


# Directories are created private (0o700) and only widened to the caller's
# usual mode after they pass `_claim_directory`. A replacement not made with
# this exact mode is detected; see README "仮定（PROVISIONAL）" for what
# cannot be detected.
_PRIVATE_DIR_MODE = 0o700
_FINAL_DIR_MODE = 0o755
_FILE_MODE = 0o644


def _current_umask():
    # POSIX has no read-only umask query; set and immediately restore.
    mask = os.umask(0o022)
    os.umask(mask)
    return mask


def _claim_directory(parent_fd, name, expected_mode):
    """Open the directory just created by mkdir() and check it is the one we made.

    mkdir() returns no handle, so the object is bound only at open(). It is
    accepted only if it is a real directory owned by us with exactly the mode
    mkdir() produces (`_PRIVATE_DIR_MODE & ~umask`, passed as `expected_mode`),
    empty, and still the object at `name`. Otherwise the run stops before
    writing anything into it. Only after acceptance is the mode set to exactly
    0o700 through the held handle, so a restrictive umask (e.g. 0o277, which
    makes mkdir() produce 0o500) cannot block our own writes. The mode is never
    changed before the object is accepted. Returns (fd, identity).
    """
    try:
        fd = os.open(name, _DIR_FLAGS, dir_fd=parent_fd)
    except OSError:
        raise GenerationError("output_raced") from None
    try:
        info = os.fstat(fd)
        if (not stat.S_ISDIR(info.st_mode)
                or info.st_uid != os.geteuid()
                or stat.S_IMODE(info.st_mode) != expected_mode
                or os.listdir(fd)
                or _current_identity(parent_fd, name) != _identity(info)):
            raise GenerationError("output_raced")
        if expected_mode != _PRIVATE_DIR_MODE:
            os.fchmod(fd, _PRIVATE_DIR_MODE)
        return fd, _identity(info)
    except BaseException:
        os.close(fd)
        raise


def _write_all(fd, data):
    view = memoryview(data)
    while view:
        view = view[os.write(fd, view):]


def _label(entry):
    label, _, _, _, identity = entry
    return label + "/" if stat.S_ISDIR(identity[2]) else label


def generate(inputs, dest_root, source_root=REPO_ROOT, contract=None):
    """Create the Level 0 template in a new directory `dest_root/inputs["output_dir"]`.

    `source_root` supplies the contract's schema; `dest_root` is the selected
    checkout that receives the output and must contain the SDK.

    Nothing is ever deleted, renamed or overwritten. On failure the objects this
    run created stay in place and are reported (`created` / `left_behind`;
    `removed` is always empty), so a concurrent replacement can never turn a
    cleanup into deletion of foreign data. Raises GenerationError on failure;
    KeyboardInterrupt and other non-OS exceptions propagate unchanged.
    """
    contract = contract_module.load_contract() if contract is None else contract
    # Read once, before the first mkdir(); the creation mode of every directory
    # is derived from it, so the check and the creation cannot disagree.
    umask = _current_umask()
    created_dir_mode = _PRIVATE_DIR_MODE & ~umask
    if not created_dir_mode & stat.S_IRUSR:
        # mkdir() would make a directory even its owner cannot open(), so it
        # could not be bound to a handle or verified. Refuse before creating
        # anything rather than change the process-wide umask.
        raise GenerationError("unsupported_umask")
    # Honor the umask, but never leave the owner without rwx on its own output.
    final_dir_mode = (_FINAL_DIR_MODE & ~umask) | _PRIVATE_DIR_MODE
    files, root_fd, parent_fd, name = _preflight(inputs, dest_root, source_root, contract)
    output_dir = inputs["output_dir"]
    parent_parts = PurePosixPath(output_dir).parts[:-1]
    # (label, parent_fd, name, fd, identity) in creation order. Every fd stays
    # open until the run ends, so a recorded (dev, inode) cannot be reused by
    # another object while it is still compared against.
    created = []
    rejected = []          # mkdir() succeeded but the opened object was not accepted
    open_fds = [parent_fd, root_fd]
    directory_fds = {}

    def make_directory(parent, child_name, label):
        os.mkdir(child_name, _PRIVATE_DIR_MODE, dir_fd=parent)   # fails if anything exists
        try:
            child, identity = _claim_directory(parent, child_name, created_dir_mode)
        except GenerationError:
            rejected.append(label + "/")
            raise
        open_fds.append(child)
        created.append((label, parent, child_name, child, identity))
        return child

    def verify_published():
        # The output must still be reachable at the requested path and consist
        # of exactly the objects we hold. A mismatch is reported, never repaired.
        try:
            reached = _walk_directories(root_fd, parent_parts, "output_parent")
        except GenerationError:
            raise GenerationError("output_raced") from None
        try:
            same_parent = _identity(os.fstat(reached)) == _identity(os.fstat(parent_fd))
        finally:
            os.close(reached)
        if not same_parent:
            raise GenerationError("output_raced")
        children = {}
        for _, parent, child_name, fd, identity in created:
            if (_current_identity(parent, child_name) != identity
                    or os.fstat(fd).st_nlink == 0):
                raise GenerationError("output_raced")
            children.setdefault(parent, set()).add(child_name)
        for _, _, _, fd, identity in created:
            if stat.S_ISDIR(identity[2]) and set(os.listdir(fd)) != children.get(fd, set()):
                raise GenerationError("output_raced")

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
                                 _FILE_MODE, dir_fd=parent)
            open_fds.append(descriptor)
            # Identity from the creating (O_EXCL) handle, never from the name.
            created.append((output_dir + "/" + relative, parent, target.name, descriptor,
                            _identity(os.fstat(descriptor))))
            _write_all(descriptor, text.encode("utf-8"))
        verify_published()
        for _, _, _, fd, identity in created:
            if stat.S_ISDIR(identity[2]):
                os.fchmod(fd, final_dir_mode)        # through the held handle only
    except (GenerationError, OSError) as error:
        code = error.code if isinstance(error, GenerationError) else (
            "write_failed:" + errno.errorcode.get(error.errno, "unknown"))
        labels = [_label(entry) for entry in created]
        raise GenerationError(code, labels, (), labels + rejected) from None
    finally:
        for descriptor in open_fds:
            try:
                os.close(descriptor)
            except OSError:
                pass
    return {"status": "created", "scope": "host_files_only", "built": False,
            "validated": False, "output_dir": output_dir,
            "created": [entry[0] for entry in created if not stat.S_ISDIR(entry[4][2])],
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
    except KeyboardInterrupt:
        # Nothing is deleted; whatever was created stays under output_dir.
        print(json.dumps({"status": "failed", "reason": "interrupted", "built": False,
                          "output_dir": args.output_dir, "created": "unknown",
                          "removed": [], "left_behind": "inspect output_dir"},
                         ensure_ascii=False, sort_keys=True))
        return 130
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
