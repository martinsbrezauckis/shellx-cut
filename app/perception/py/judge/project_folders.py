"""Reach a project file through ordinary folders only.

A project may come from someone else and may hold a link or a Windows junction
at any folder name, for example ``.scratch/redirect`` pointing outside the
project. A path inside a project (the outermost folder on the path that holds
``ops.jsonl`` or ``project.json`` or is named ``*.cutproj``) must
therefore reach its file through ordinary folders from that project folder
down: a link or junction anywhere below the project folder is refused, so it
cannot send a read or a write outside the project. Links above the project
folder (``/tmp`` or ``/var`` on macOS, a linked Videos folder) are the machine
owner's and are followed. Outside a project only the file's own folder is held
to the rule for writes, as before.

On Linux and macOS each folder below the project folder is opened relative to
the previous one without following a link, and the caller reads, creates and
renames the file relative to the last folder's handle, so the folders cannot
be swapped between the check and the use. Python on Windows has no
handle-relative open: there every folder is inspected by name (symbolic links
and junctions are name-surrogate reparse points) immediately before the file
is used, and a folder replaced by a junction in that moment is not detected.
"""

from __future__ import annotations

import os
import stat

_REPARSE_POINT = 0x400  # FILE_ATTRIBUTE_REPARSE_POINT
# Name-surrogate bit of a Windows reparse tag (IsReparseTagNameSurrogate).
NAME_SURROGATE_BIT = 0x20000000
# Files that make a folder a project folder (the engine's ProjectStore::open).
_PROJECT_MARKERS = ("ops.jsonl", "project.json")
# Folders are opened by handle where the platform can do it without following
# a link (Linux, macOS); elsewhere they are inspected by name.
FOLDER_HANDLES = (os.name != "nt" and hasattr(os, "O_DIRECTORY") and hasattr(os, "O_NOFOLLOW")
                  and {os.open, os.stat, os.rename, os.unlink} <= os.supports_dir_fd)


class RefusedProjectFile(ValueError):
    """A project file the review would use is not an ordinary file.

    The message is plain text written for the person running the review.
    """


def is_name_surrogate_tag(tag: int) -> bool:
    """Whether a Windows reparse tag stands for another named entity."""
    return bool(tag & NAME_SURROGATE_BIT)


def names_another_entry(info: os.stat_result) -> bool:
    """True when an lstat() entry names another entry.

    That is a symbolic link, or on Windows a reparse point whose tag is a name
    surrogate (symbolic links, junctions/mount points). Other reparse points,
    such as cloud-file placeholders kept by a sync client or compressed files,
    are the file or folder itself. A reparse point whose tag could not be read
    (reported as 0) is refused.
    """
    if stat.S_ISLNK(info.st_mode):
        return True
    return attributes_name_another_entry(getattr(info, "st_file_attributes", 0),
                                         getattr(info, "st_reparse_tag", 0))


def attributes_name_another_entry(attributes: int, tag: int) -> bool:
    """The same rule from raw Windows file attributes and reparse tag, as an
    open handle reports them: only a name surrogate, or a reparse point whose
    tag could not be read (0), names another entry."""
    if not attributes & _REPARSE_POINT:
        return False
    return tag == 0 or is_name_surrogate_tag(tag)


def is_project_folder(path: str) -> bool:
    if os.path.basename(path).endswith(".cutproj"):
        return True
    return any(os.path.lexists(os.path.join(path, marker)) for marker in _PROJECT_MARKERS)


def route(folder: str, own_folder: bool) -> tuple[str, list[str]]:
    """Split an absolute folder into a start folder and the names below it
    that must be ordinary folders.

    Inside a project the start is the outermost project folder (which may
    itself be reached through a link the owner made). Outside a project the
    start is the folder's parent and only the folder itself is checked when
    ``own_folder`` is set, otherwise nothing is checked.
    """
    drive, rest = os.path.splitdrive(folder)
    names = [name for name in rest.replace("\\", "/").split("/") if name]
    prefix = drive + os.sep
    for index, name in enumerate(names):
        prefix = os.path.join(prefix, name)
        if is_project_folder(prefix):
            return prefix, names[index + 1:]
    parent = os.path.dirname(folder)
    if own_folder and parent != folder:
        return parent, [os.path.basename(folder)]
    return folder, []


def _not_a_folder(label: str, path: str, folder: str) -> RefusedProjectFile:
    return RefusedProjectFile(
        f"the folder {folder} on the path of the {label} {path} is a link or not an ordinary "
        "folder; links inside a project are not followed")


def open_folder(folder: str, own_folder: bool, label: str, path: str) -> int | None:
    """Open ``folder`` through ordinary folders only (Linux, macOS).

    Returns a directory descriptor, or None when a folder on the way does not
    exist yet (nothing can be planted inside it).
    """
    start, names = route(folder, own_folder)
    flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_CLOEXEC", 0)
    try:
        descriptor = os.open(start, flags)
    except FileNotFoundError:
        return None
    current = start
    for name in names:
        current = os.path.join(current, name)
        try:
            child = os.open(name, flags | os.O_NOFOLLOW, dir_fd=descriptor)
        except FileNotFoundError:
            os.close(descriptor)
            return None
        except OSError as error:
            os.close(descriptor)
            # ELOOP: a link; ENOTDIR: a file.
            raise _not_a_folder(label, path, current) from error
        os.close(descriptor)
        descriptor = child
    return descriptor


def check_folders_by_name(folder: str, own_folder: bool, label: str, path: str) -> bool:
    """Inspect every folder below the start by name (Windows). False when a
    folder on the way does not exist yet."""
    start, names = route(folder, own_folder)
    current = start
    for name in names:
        current = os.path.join(current, name)
        try:
            info = os.lstat(current)
        except FileNotFoundError:
            return False
        if names_another_entry(info) or not stat.S_ISDIR(info.st_mode):
            raise _not_a_folder(label, path, current)
    return os.path.isdir(start)
