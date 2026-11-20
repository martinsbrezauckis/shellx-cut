"""Read and write render-review files without following project-supplied links.

A project, including its receipts and its ``.scratch`` folder, may come from
someone else. Every file the review reads from that tree, and every file it
writes there, therefore has to be an ordinary file that belongs to that path:

* reads accept only a regular file that is not a symbolic link, a Windows
  name-surrogate reparse point (symbolic link, junction), or a file shared
  with another path (a hard link), open it
  without following a link, confirm the opened file is the one inspected, and
  stop at a byte limit;
* writes create a new private temporary file beside the destination and move
  it into place, so an existing link at the destination is replaced as a name
  rather than followed. An existing destination that is not an ordinary file is
  refused before any review work starts.

Every folder on the way is checked as well; see ``project_folders``.

The engine applies the same rules before it starts an adapter; these helpers
keep the adapters safe when they are run directly.
"""

from __future__ import annotations

import hashlib
import os
import secrets
import stat

# The adapters import this module with judge/ on sys.path; the other runners
# import it as judge.judge_files.
try:
    from . import project_folders as _folders
except ImportError:
    import project_folders as _folders

NAME_SURROGATE_BIT = _folders.NAME_SURROGATE_BIT
RefusedProjectFile = _folders.RefusedProjectFile
is_name_surrogate_tag = _folders.is_name_surrogate_tag
names_another_entry = _folders.names_another_entry
_FOLDER_HANDLES = _folders.FOLDER_HANDLES
_check_folders_by_name = _folders.check_folders_by_name
_open_folder = _folders.open_folder

# Analysis reports and review results are small JSON documents. Their ceiling
# matches the engine's perception-report limit.
MAX_REPORT_BYTES = 64 * 1024 * 1024


def _ordinary(info: os.stat_result) -> bool:
    """An lstat() entry that is a regular file with one name."""
    return (stat.S_ISREG(info.st_mode) and not names_another_entry(info)
            and info.st_nlink == 1)


def _ordinary_opened(info: os.stat_result) -> bool:
    """An fstat() of an opened file: regular with one name. The entry itself
    was already checked by name; a handle never stands for another entry."""
    return stat.S_ISREG(info.st_mode) and info.st_nlink == 1


def _same_file(before: os.stat_result, opened: os.stat_result) -> bool:
    """Whether the opened file is the one inspected by name.

    POSIX opens refuse a final link (O_NOFOLLOW) and the device/inode pair
    identifies the file. Windows stat identities are not guaranteed to use the
    same file-id form for a path and a handle, so there the check relies on
    the name inspection plus the opened file's own type and link count.
    """
    if os.name == "nt":
        return True
    return (opened.st_dev, opened.st_ino) == (before.st_dev, before.st_ino)


def require_plain_directory(path: str, label: str) -> None:
    """Refuse a directory path that is a link, a reparse point, or not a
    directory, or that is reached through a link inside a project."""
    folder = os.path.abspath(path)
    if _FOLDER_HANDLES:
        descriptor = _open_folder(folder, True, label, folder)
        if descriptor is not None:
            os.close(descriptor)
    else:
        _check_folders_by_name(folder, True, label, folder)


def _read_flags() -> int:
    return (os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
            | getattr(os, "O_NONBLOCK", 0))


def _read_open(path: str, label: str) -> tuple[os.stat_result, int]:
    """lstat and open an ordinary file without following a link at its name
    or at a folder inside a project."""
    path = os.path.abspath(path)
    folder, name = os.path.split(path)
    if not _FOLDER_HANDLES:
        if not _check_folders_by_name(folder, False, label, path):
            raise FileNotFoundError(path)
        before = os.lstat(path)
        if not _ordinary(before):
            raise RefusedProjectFile(f"{label} {path} is a link or not an ordinary file")
        return before, os.open(path, _read_flags())
    parent = _open_folder(folder, False, label, path)
    if parent is None:
        raise FileNotFoundError(path)
    try:
        before = os.stat(name, dir_fd=parent, follow_symlinks=False)
        if not _ordinary(before):
            raise RefusedProjectFile(f"{label} {path} is a link or not an ordinary file")
        return before, os.open(name, _read_flags(), dir_fd=parent)
    finally:
        os.close(parent)


def read_plain_file(path: str, label: str, limit: int = MAX_REPORT_BYTES) -> bytes:
    """Return the bytes of an ordinary single-link file, or refuse it.

    FileNotFoundError is raised unchanged so callers can treat a missing
    optional file as absent.
    """
    before, descriptor = _read_open(path, label)
    try:
        opened = os.fstat(descriptor)
        if not _ordinary_opened(opened) or not _same_file(before, opened):
            raise RefusedProjectFile(
                f"{label} {path} changed while it was being opened")
        if opened.st_size > limit:
            raise RefusedProjectFile(f"{label} {path} is larger than {limit} bytes")
        chunks = []
        remaining = limit + 1
        while remaining > 0:
            chunk = os.read(descriptor, min(remaining, 1 << 20))
            if not chunk:
                break
            chunks.append(chunk)
            remaining -= len(chunk)
        data = b"".join(chunks)
    finally:
        os.close(descriptor)
    if len(data) > limit:
        raise RefusedProjectFile(f"{label} {path} is larger than {limit} bytes")
    return data


def _check_leaf(info_of, path: str, label: str) -> None:
    try:
        info = info_of()
    except FileNotFoundError:
        return
    if not _ordinary(info):
        raise RefusedProjectFile(f"{label} {path} is a link or not an ordinary file")


def require_replaceable(path: str, label: str) -> None:
    """Refuse a destination whose folders or existing file are not ordinary.

    Call this before review work starts so a planted link stops the review
    instead of being discovered after a provider was used. A folder that does
    not exist yet (for example a bundle folder the adapter creates) holds
    nothing that could be planted; the final write checks again.
    """
    path = os.path.abspath(path)
    folder, name = os.path.split(path)
    if not _FOLDER_HANDLES:
        if _check_folders_by_name(folder, True, label, path):
            _check_leaf(lambda: os.lstat(path), path, label)
        return
    parent = _open_folder(folder, True, label, path)
    if parent is None:
        return
    try:
        _check_leaf(lambda: os.stat(name, dir_fd=parent, follow_symlinks=False), path, label)
    finally:
        os.close(parent)


def _write_all(descriptor: int, data: bytes) -> None:
    try:
        view = memoryview(data)
        while view:
            view = view[os.write(descriptor, view):]
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def write_replacing(path: str, data: bytes, label: str) -> None:
    """Write ``data`` to a new temporary file and move it over ``path``.

    The move replaces the destination name; it never writes through an
    existing link or into a file shared with another path, and never through
    a link at a folder inside a project.
    """
    path = os.path.abspath(path)
    folder, name = os.path.split(path)
    flags = (os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)
             | getattr(os, "O_NOFOLLOW", 0))
    temporary = f".{name}.{os.getpid()}.{secrets.token_hex(8)}.tmp"
    if not _FOLDER_HANDLES:
        require_replaceable(path, label)
        temporary = os.path.join(folder, temporary)
        _write_all(os.open(temporary, flags, 0o600), data)
        try:
            os.replace(temporary, path)
        except BaseException:
            try:
                os.unlink(temporary)
            except OSError:
                pass
            raise
        return
    parent = _open_folder(folder, True, label, path)
    if parent is None:
        raise FileNotFoundError(folder)
    try:
        _check_leaf(lambda: os.stat(name, dir_fd=parent, follow_symlinks=False), path, label)
        _write_all(os.open(temporary, flags, 0o600, dir_fd=parent), data)
        try:
            os.rename(temporary, name, src_dir_fd=parent, dst_dir_fd=parent)
        except BaseException:
            try:
                os.unlink(temporary, dir_fd=parent)
            except OSError:
                pass
            raise
    finally:
        os.close(parent)


def write_text_replacing(path: str, text: str, label: str) -> None:
    write_replacing(path, text.encode("utf-8"), label)


def content_hash(path: str) -> str:
    """Hash a render the way the engine records it on the receipt.

    Render receipts and their analysis files carry the full SHA-256 of the
    rendered file as ``sha256:<hex>``. The render may live in a chosen export
    folder, so it is not held to the single-link rule, but a link at the
    render path itself is refused.
    """
    before = os.lstat(path)
    if names_another_entry(before) or not stat.S_ISREG(before.st_mode):
        raise RefusedProjectFile(f"the render {path} is a link or not an ordinary file")
    descriptor = os.open(path, _read_flags())
    try:
        opened = os.fstat(descriptor)
        if not stat.S_ISREG(opened.st_mode) or not _same_file(before, opened):
            raise RefusedProjectFile(f"the render {path} changed while it was being opened")
        digest = hashlib.sha256()
        while chunk := os.read(descriptor, 1 << 20):
            digest.update(chunk)
        return "sha256:" + digest.hexdigest()
    finally:
        os.close(descriptor)
