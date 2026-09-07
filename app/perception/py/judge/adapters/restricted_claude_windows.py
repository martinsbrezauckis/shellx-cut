"""Windows frame-root copier for the restricted Claude render judge.
The provider receives only a fresh directory containing selected frames. A
source frame is opened first, inspected through that exact handle, and read
only after a non-reparse, single-link destination has been created below a
private handle-rooted directory. Path checks are never used to reopen a frame.
"""
from __future__ import annotations
import ctypes
import os
import secrets
import shutil
from ctypes import wintypes
from functools import lru_cache
FILE_ATTRIBUTE_DIRECTORY = 0x10
FILE_ATTRIBUTE_NORMAL = 0x80
FILE_ATTRIBUTE_REPARSE_POINT = 0x400
FILE_FLAG_BACKUP_SEMANTICS = 0x02000000
FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000
FILE_SHARE_ALL = 7
FILE_READ_DATA = 1
FILE_WRITE_DATA = 2
FILE_ADD_FILE = 2
FILE_ADD_SUBDIRECTORY = 4
FILE_TRAVERSE = 0x20
FILE_READ_ATTRIBUTES = 0x80
SYNCHRONIZE = 0x100000
OPEN_EXISTING = 3
FILE_ATTRIBUTE_TAG_INFO = 9
FILE_STANDARD_INFO = 1
FILE_ID_INFO = 18
FILE_CASE_SENSITIVE_INFO = 23
FILE_TYPE_DISK = 1
FILE_OPEN = 1
FILE_CREATE = 2
FILE_OPEN_IF = 3
FILE_DIRECTORY_FILE = 1
FILE_NON_DIRECTORY_FILE = 0x40
FILE_SYNCHRONOUS_IO_NONALERT = 0x20
OBJ_CASE_INSENSITIVE = 0x40
OBJ_DONT_REPARSE = 0x1000
TOKEN_QUERY = 8
TOKEN_USER = 1
SDDL_REVISION_1 = 1
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
class _FileAttributeTagInfo(ctypes.Structure):
    _fields_ = [("attributes", wintypes.DWORD), ("reparse_tag", wintypes.DWORD)]
class _FileStandardInfo(ctypes.Structure):
    _fields_ = [("allocation_size", ctypes.c_longlong), ("end_of_file", ctypes.c_longlong),
                ("number_of_links", wintypes.DWORD), ("delete_pending", ctypes.c_ubyte),
                ("directory", ctypes.c_ubyte)]
class _FileId128(ctypes.Structure):
    _fields_ = [("identifier", ctypes.c_byte * 16)]
class _FileIdInfo(ctypes.Structure):
    _fields_ = [("volume_serial", ctypes.c_ulonglong), ("file_id", _FileId128)]
class _FileCaseSensitiveInfo(ctypes.Structure):
    _fields_ = [("flags", wintypes.DWORD)]
class _UnicodeString(ctypes.Structure):
    _fields_ = [("length", wintypes.WORD), ("maximum_length", wintypes.WORD),
                ("buffer", wintypes.LPWSTR)]
class _ObjectAttributes(ctypes.Structure):
    _fields_ = [("length", wintypes.ULONG), ("root_directory", wintypes.HANDLE),
                ("object_name", ctypes.POINTER(_UnicodeString)), ("attributes", wintypes.ULONG),
                ("security_descriptor", wintypes.LPVOID),
                ("security_quality_of_service", wintypes.LPVOID)]
class _IoStatusBlock(ctypes.Structure):
    _fields_ = [("status", ctypes.c_long), ("information", ctypes.c_size_t)]
class _TokenUser(ctypes.Structure):
    _fields_ = [("sid", wintypes.LPVOID), ("attributes", wintypes.DWORD)]
def available() -> tuple[bool, str | None]:
    """Return whether this host can perform handle-rooted frame copying."""
    if os.name != "nt":
        return False, "Windows handle-root confinement is unavailable on this host"
    try:
        _api()
    except (AttributeError, OSError) as exc:
        return False, f"Windows handle-root confinement is unavailable: {exc}"
    return True, None
@lru_cache(maxsize=1)
def _api():
    if os.name != "nt":
        raise OSError("Win32 APIs are unavailable")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    advapi = ctypes.WinDLL("advapi32", use_last_error=True)
    ntdll = ctypes.WinDLL("ntdll", use_last_error=True)
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                   wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD,
                                   wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes, kernel.CloseHandle.restype = [wintypes.HANDLE], wintypes.BOOL
    kernel.GetFileInformationByHandleEx.argtypes = [wintypes.HANDLE, ctypes.c_int,
                                                    wintypes.LPVOID, wintypes.DWORD]
    kernel.GetFileInformationByHandleEx.restype = wintypes.BOOL
    kernel.GetFinalPathNameByHandleW.argtypes = [wintypes.HANDLE, wintypes.LPWSTR,
                                                  wintypes.DWORD, wintypes.DWORD]
    kernel.GetFinalPathNameByHandleW.restype = wintypes.DWORD
    kernel.GetFileType.argtypes, kernel.GetFileType.restype = [wintypes.HANDLE], wintypes.DWORD
    kernel.ReadFile.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD,
                                ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
    kernel.ReadFile.restype = wintypes.BOOL
    kernel.WriteFile.argtypes = [wintypes.HANDLE, wintypes.LPCVOID, wintypes.DWORD,
                                 ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
    kernel.WriteFile.restype = wintypes.BOOL
    kernel.GetCurrentProcess.argtypes, kernel.GetCurrentProcess.restype = [], wintypes.HANDLE
    kernel.LocalFree.argtypes, kernel.LocalFree.restype = [wintypes.HLOCAL], wintypes.HLOCAL
    advapi.OpenProcessToken.argtypes = [wintypes.HANDLE, wintypes.DWORD,
                                        ctypes.POINTER(wintypes.HANDLE)]
    advapi.OpenProcessToken.restype = wintypes.BOOL
    advapi.GetTokenInformation.argtypes = [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID,
                                           wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
    advapi.GetTokenInformation.restype = wintypes.BOOL
    advapi.ConvertSidToStringSidW.argtypes = [wintypes.LPVOID, ctypes.POINTER(wintypes.LPWSTR)]
    advapi.ConvertSidToStringSidW.restype = wintypes.BOOL
    advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [
        wintypes.LPCWSTR, wintypes.DWORD, ctypes.POINTER(wintypes.LPVOID),
        ctypes.POINTER(wintypes.DWORD)]
    advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.restype = wintypes.BOOL
    ntdll.NtCreateFile.argtypes = [ctypes.POINTER(wintypes.HANDLE), wintypes.DWORD,
                                   ctypes.POINTER(_ObjectAttributes), ctypes.POINTER(_IoStatusBlock),
                                   wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD,
                                   wintypes.DWORD, wintypes.LPVOID, wintypes.ULONG]
    ntdll.NtCreateFile.restype = ctypes.c_long
    return kernel, advapi, ntdll
def _error(action: str) -> OSError:
    code = ctypes.get_last_error()
    return OSError(code, f"{action}: {ctypes.FormatError(code)}")
def _handle(value: object) -> int:
    raw = getattr(value, "value", value)
    return int(raw) if raw is not None else 0
def _close(handle: int) -> None:
    if handle not in (0, INVALID_HANDLE_VALUE) and not _api()[0].CloseHandle(handle):
        raise _error("closing handle")
def _open(path: str, access: int, *, directory: bool, share: int = FILE_SHARE_ALL) -> int:
    flags = FILE_FLAG_OPEN_REPARSE_POINT | (FILE_FLAG_BACKUP_SEMANTICS if directory else 0)
    handle = _handle(_api()[0].CreateFileW(path, access, share, None, OPEN_EXISTING, flags, None))
    if handle == INVALID_HANDLE_VALUE:
        raise _error(f"opening {path!r}")
    return handle
def _query(handle: int, kind: int, structure: type[ctypes.Structure]):
    value = structure()
    if not _api()[0].GetFileInformationByHandleEx(handle, kind, ctypes.byref(value), ctypes.sizeof(value)):
        raise _error("querying file handle information")
    return value
def _final_path(handle: int) -> str:
    kernel, size = _api()[0], 1024
    while True:
        buffer = ctypes.create_unicode_buffer(size)
        written = kernel.GetFinalPathNameByHandleW(handle, buffer, size, 0)
        if not written:
            raise _error("resolving final handle path")
        if written < size:
            return os.path.normcase(os.path.normpath(buffer.value)).rstrip("\\/")
        size = written + 1
def _literal_windows_path(path: str) -> str:
    """Normalize only the Win32 extended namespace for identity comparison."""
    value = os.path.normpath(os.path.abspath(path)).rstrip("\\/")
    extended_dos = "\\\\" + "?\\"
    extended_unc = extended_dos + "UNC\\"
    if value.startswith(extended_unc):
        value = "\\\\" + value[len(extended_unc):]
    elif value.startswith(extended_dos):
        value = value[len(extended_dos):]
    return os.path.normcase(value)
def _identity(handle: int) -> tuple[int, bytes]:
    info = _query(handle, FILE_ID_INFO, _FileIdInfo)
    return info.volume_serial, bytes(info.file_id.identifier)
def _inside(root: str, candidate: str) -> bool:
    return candidate.casefold().startswith((root.rstrip("\\/") + "\\").casefold())
def _safe_relative_frame_path(value: object) -> str:
    if not isinstance(value, str) or not value or os.path.isabs(value) or os.path.splitdrive(value)[0]:
        raise ValueError("frame path must be a non-empty relative path")
    normalized, pieces = os.path.normpath(value), os.path.normpath(value).split(os.sep)
    if (normalized in (".", "..") or normalized.startswith(".." + os.sep)
            or not pieces or pieces[0] != "frames" or any(not p or ":" in p for p in pieces)):
        raise ValueError(f"frame path escapes the staging bundle: {value!r}")
    return normalized
def _validate(handle: int, *, directory: bool, root: tuple[str, int] | None = None,
              single_link: bool = False) -> tuple[str, tuple[int, bytes]]:
    tag, standard = (_query(handle, FILE_ATTRIBUTE_TAG_INFO, _FileAttributeTagInfo),
                     _query(handle, FILE_STANDARD_INFO, _FileStandardInfo))
    if (_api()[0].GetFileType(handle) != FILE_TYPE_DISK or tag.attributes & FILE_ATTRIBUTE_REPARSE_POINT
            or bool(standard.directory) != directory or (single_link and standard.number_of_links != 1)):
        raise ValueError("restricted frame object is not a permitted regular filesystem object")
    if directory and _query(handle, FILE_CASE_SENSITIVE_INFO, _FileCaseSensitiveInfo).flags:
        raise ValueError("case-sensitive directories are unsupported by restricted Windows frame paths")
    path, identity = _final_path(handle), _identity(handle)
    if root is not None and (identity[0] != root[1] or not _inside(root[0], path)):
        raise ValueError("resolved restricted path escapes its retained root")
    return path, identity
def _read(handle: int, size: int = 1 << 20) -> bytes:
    buffer, count = ctypes.create_string_buffer(size), wintypes.DWORD()
    if not _api()[0].ReadFile(handle, buffer, size, ctypes.byref(count), None):
        raise _error("reading validated frame handle")
    return buffer.raw[:count.value]
def _write(handle: int, value: bytes) -> None:
    offset = 0
    while offset < len(value):
        buffer, count = ctypes.create_string_buffer(value[offset:]), wintypes.DWORD()
        if not _api()[0].WriteFile(handle, buffer, len(buffer) - 1, ctypes.byref(count), None) or not count.value:
            raise _error("writing restricted frame copy")
        offset += count.value
def _nt_create(parent: int, name: str, *, directory: bool, disposition: int,
               security: int | None = None, share: int = FILE_SHARE_ALL) -> int:
    if not name or name in (".", "..") or any(char in name for char in "\\/:"):
        raise ValueError("invalid restricted destination component")
    text = ctypes.create_unicode_buffer(name)
    string = _UnicodeString(len(name) * ctypes.sizeof(ctypes.c_wchar),
                            (len(name) + 1) * ctypes.sizeof(ctypes.c_wchar),
                            ctypes.cast(text, wintypes.LPWSTR))
    attributes = _ObjectAttributes(ctypes.sizeof(_ObjectAttributes), parent, ctypes.pointer(string),
                                   OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE, security, None)
    handle, status = wintypes.HANDLE(), _IoStatusBlock()
    if disposition == FILE_OPEN:
        access = FILE_READ_ATTRIBUTES | SYNCHRONIZE | (FILE_TRAVERSE if directory else FILE_READ_DATA)
    else:
        access = FILE_READ_ATTRIBUTES | SYNCHRONIZE | (
            FILE_TRAVERSE | FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY if directory else FILE_WRITE_DATA)
    options = FILE_SYNCHRONOUS_IO_NONALERT | FILE_FLAG_OPEN_REPARSE_POINT
    options |= FILE_DIRECTORY_FILE if directory else FILE_NON_DIRECTORY_FILE
    result = _api()[2].NtCreateFile(ctypes.byref(handle), access, ctypes.byref(attributes),
                                    ctypes.byref(status), None,
                                    FILE_ATTRIBUTE_DIRECTORY if directory else FILE_ATTRIBUTE_NORMAL,
                                    share, disposition, options, None, 0)
    if ctypes.c_int32(result).value < 0:
        raise OSError(f"NtCreateFile({name!r}) failed with NTSTATUS 0x{result & 0xffffffff:08x}")
    return _handle(handle)
def _current_user_security_descriptor() -> int:
    kernel, advapi, _ = _api()
    token = wintypes.HANDLE()
    if not advapi.OpenProcessToken(kernel.GetCurrentProcess(), TOKEN_QUERY, ctypes.byref(token)):
        raise _error("opening current process token")
    try:
        required = wintypes.DWORD()
        advapi.GetTokenInformation(token, TOKEN_USER, None, 0, ctypes.byref(required))
        if not required.value:
            raise _error("sizing current user token")
        data = ctypes.create_string_buffer(required.value)
        if not advapi.GetTokenInformation(token, TOKEN_USER, data, required, ctypes.byref(required)):
            raise _error("reading current user token")
        sid = ctypes.cast(data, ctypes.POINTER(_TokenUser)).contents.sid
        text = wintypes.LPWSTR()
        if not advapi.ConvertSidToStringSidW(sid, ctypes.byref(text)):
            raise _error("formatting current user SID")
        try:
            sddl = f"O:{text.value}D:P(A;OICI;FA;;;{text.value})"
        finally:
            kernel.LocalFree(text)
        descriptor, unused = wintypes.LPVOID(), wintypes.DWORD()
        if not advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl, SDDL_REVISION_1, ctypes.byref(descriptor), ctypes.byref(unused)):
            raise _error("creating private output security descriptor")
        return _handle(descriptor)
    finally:
        _close(_handle(token))
def _private_execution_root(staging_root: str) -> tuple[str, int, tuple[int, bytes]]:
    parent = os.path.dirname(staging_root)
    parent_handle = _open(parent, FILE_READ_ATTRIBUTES | FILE_TRAVERSE | FILE_ADD_SUBDIRECTORY,
                          directory=True, share=0)
    try:
        parent_final, parent_identity = _validate(parent_handle, directory=True)
        if not _inside(parent_final, staging_root):
            raise ValueError("staging root is not under its retained parent handle")
        descriptor = _current_user_security_descriptor()
        try:
            root_handle = _nt_create(parent_handle, "cli_judge_restricted_" + secrets.token_hex(16),
                                     directory=True, disposition=FILE_CREATE, security=descriptor)
        finally:
            _api()[0].LocalFree(descriptor)
    finally:
        _close(parent_handle)
    root_final, root_identity = _validate(root_handle, directory=True,
                                          root=(parent_final, parent_identity[0]))
    return root_final, root_handle, root_identity
def _destination_parent(root_handle: int, root: tuple[str, int], parts: list[str]) -> int:
    current, owned = root_handle, False
    try:
        for part in parts:
            child = _nt_create(current, part, directory=True, disposition=FILE_OPEN_IF)
            _validate(child, directory=True, root=root)
            if owned:
                _close(current)
            current, owned = child, True
        return current
    except Exception:
        if owned:
            _close(current)
        raise
def _source_handle(root_handle: int, root: tuple[str, int], parts: list[str]) -> int:
    current, owned = root_handle, False
    try:
        for index, part in enumerate(parts):
            child = _nt_create(current, part, directory=index != len(parts) - 1,
                               disposition=FILE_OPEN, share=FILE_SHARE_ALL)
            _validate(child, directory=index != len(parts) - 1, root=root,
                      single_link=index == len(parts) - 1)
            if owned:
                _close(current)
            current, owned = child, True
        return current
    except Exception:
        if owned:
            _close(current)
        raise
def _copy_validated_source(source_root_handle: int, source_parts: list[str], parent: int, name: str,
                           source_root: tuple[str, int], output_root: tuple[str, int]) -> None:
    source_handle = _source_handle(source_root_handle, source_root, source_parts)
    try:
        _validate(source_handle, directory=False, root=source_root, single_link=True)
        destination = _nt_create(parent, name, directory=False, disposition=FILE_CREATE)
        try:
            _validate(destination, directory=False, root=output_root, single_link=True)
            while chunk := _read(source_handle):
                _write(destination, chunk)
        finally:
            _close(destination)
    finally:
        _close(source_handle)
def create_frame_only_bundle(staging_bundle: str, frame_relpaths: list[object]) -> str:
    """Return a private Windows root containing only validated copied frames."""
    ok, reason = available()
    if not ok:
        raise ValueError(reason)
    literal_staging = _literal_windows_path(staging_bundle)
    staging_handle = _open(staging_bundle, FILE_READ_ATTRIBUTES | FILE_TRAVERSE, directory=True, share=0)
    try:
        staging_root, staging_identity = _validate(staging_handle, directory=True)
        if _literal_windows_path(staging_root) != literal_staging:
            raise ValueError("staging root resolves through an ancestor reparse point")
        execution_root, root_handle, root_identity = _private_execution_root(staging_root)
        try:
            source_root, output_root = (staging_root, staging_identity[0]), (execution_root, root_identity[0])
            seen: set[str] = set()
            for value in frame_relpaths:
                relative = _safe_relative_frame_path(value)
                if relative in seen:
                    raise ValueError(f"duplicate restricted frame path: {relative!r}")
                seen.add(relative)
                parts = relative.split(os.sep)
                parent = _destination_parent(root_handle, output_root, parts[:-1])
                try:
                    _copy_validated_source(staging_handle, parts, parent, parts[-1], source_root, output_root)
                finally:
                    _close(parent)
            return execution_root
        except Exception:
            shutil.rmtree(execution_root, ignore_errors=True)
            raise
        finally:
            _close(root_handle)
    finally:
        _close(staging_handle)
