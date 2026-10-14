"""Descriptor APIs for Python builds targeting older macOS SDKs."""

import os as _os
import sys


class DarwinOS:
    """Use libc when the running OS has APIs omitted by the Python build."""

    def __init__(self):
        import ctypes

        self.ctypes = ctypes
        if sys.platform != "darwin" or ctypes.sizeof(ctypes.c_void_p) != 8:
            raise RuntimeError("The Darwin descriptor adapter requires 64-bit macOS")
        self.libc = ctypes.CDLL(None, use_errno=True)
        integer, pointer, string = ctypes.c_int, ctypes.c_void_p, ctypes.c_char_p

        class Timespec(ctypes.Structure):
            _fields_ = [("seconds", ctypes.c_int64), ("nanoseconds", ctypes.c_int64)]

        class Stat(ctypes.Structure):
            _fields_ = [
                ("st_dev", ctypes.c_int32),
                ("st_mode", ctypes.c_uint16),
                ("st_nlink", ctypes.c_uint16),
                ("st_ino", ctypes.c_uint64),
                ("st_uid", ctypes.c_uint32),
                ("st_gid", ctypes.c_uint32),
                ("st_rdev", ctypes.c_int32),
                ("atime", Timespec),
                ("mtime", Timespec),
                ("ctime", Timespec),
                ("birthtime", Timespec),
                ("st_size", ctypes.c_int64),
                ("st_blocks", ctypes.c_int64),
                ("st_blksize", ctypes.c_int32),
                ("st_flags", ctypes.c_uint32),
                ("st_gen", ctypes.c_uint32),
                ("spare", ctypes.c_int32),
                ("reserved", ctypes.c_int64 * 2),
            ]

            @property
            def st_atime_ns(self):
                return self.atime.seconds * 1000000000 + self.atime.nanoseconds

            @property
            def st_mtime_ns(self):
                return self.mtime.seconds * 1000000000 + self.mtime.nanoseconds

            @property
            def st_ctime_ns(self):
                return self.ctime.seconds * 1000000000 + self.ctime.nanoseconds

        class Dirent(ctypes.Structure):
            _fields_ = [
                ("inode", ctypes.c_uint64),
                ("seek_offset", ctypes.c_uint64),
                ("record_length", ctypes.c_uint16),
                ("name_length", ctypes.c_uint16),
                ("kind", ctypes.c_uint8),
                ("name", ctypes.c_char * 1024),
            ]

        if ctypes.sizeof(Stat) != 144 or Dirent.name.offset != 21:
            raise RuntimeError("Unexpected Darwin filesystem ABI layout")
        self.Stat, self.Timespec, self.Dirent = Stat, Timespec, Dirent
        self.openat = self.function("openat", integer, [integer, string, integer])
        self.fstatat = self.function(
            "fstatat", integer, [integer, string, pointer, integer], inode64=True
        )
        self.mkdirat = self.function(
            "mkdirat", integer, [integer, string, ctypes.c_uint16]
        )
        self.readlinkat = self.function(
            "readlinkat", ctypes.c_ssize_t, [integer, string, pointer, ctypes.c_size_t]
        )
        self.unlinkat = self.function("unlinkat", integer, [integer, string, integer])
        self.renameat = self.function(
            "renameat", integer, [integer, string, integer, string]
        )
        self.linkat = self.function(
            "linkat", integer, [integer, string, integer, string, integer]
        )
        self.symlinkat = self.function("symlinkat", integer, [string, integer, string])
        self.utimensat = self.function(
            "utimensat", integer, [integer, string, pointer, integer]
        )
        self.fdopendir = self.function("fdopendir", pointer, [integer])
        self.readdir = self.function("readdir", pointer, [pointer], inode64=True)
        self.rewinddir = self.function("rewinddir", None, [pointer])
        self.closedir = self.function("closedir", integer, [pointer])

    def __getattr__(self, name):
        return getattr(_os, name)

    def function(self, name, result, arguments, inode64=False):
        function = getattr(self.libc, name + "$INODE64", None) if inode64 else None
        if function is None:
            function = getattr(self.libc, name)
        function.restype, function.argtypes = result, arguments
        return function

    def error(self, path=None):
        number = self.ctypes.get_errno()
        return OSError(number, _os.strerror(number), path)

    def checked(self, result, path=None):
        if result == -1:
            raise self.error(path)
        return result

    @staticmethod
    def _path_bytes(value):
        value = _os.fsencode(value)
        if b"\0" in value:
            raise ValueError("embedded null byte")
        return value

    @staticmethod
    def descriptor(value):
        return -2 if value is None else value  # Darwin AT_FDCWD.

    def open(self, path, flags, mode=0o777, *, dir_fd=None):
        if dir_fd is None or _os.open in _os.supports_dir_fd:
            return _os.open(path, flags, mode, dir_fd=dir_fd)
        descriptor = self.checked(
            self.openat(
                dir_fd,
                self._path_bytes(path),
                flags | _os.O_CLOEXEC,
                self.ctypes.c_uint(mode),
            ),
            path,
        )
        return descriptor

    def stat(self, path, *, dir_fd=None, follow_symlinks=True):
        if dir_fd is None or _os.stat in _os.supports_dir_fd:
            return _os.stat(path, dir_fd=dir_fd, follow_symlinks=follow_symlinks)
        result = self.Stat()
        self.checked(
            self.fstatat(
                dir_fd,
                self._path_bytes(path),
                self.ctypes.byref(result),
                0 if follow_symlinks else 0x20,
            ),
            path,
        )
        return result

    def listdir(self, path="."):
        if not isinstance(path, int) or _os.listdir in _os.supports_fd:
            return _os.listdir(path)
        descriptor = _os.dup(path)
        directory = self.fdopendir(descriptor)
        if not directory:
            error = self.error()
            _os.close(descriptor)
            raise error
        result = []
        try:
            self.rewinddir(directory)
            while True:
                self.ctypes.set_errno(0)
                entry = self.readdir(directory)
                if not entry:
                    if self.ctypes.get_errno():
                        raise self.error()
                    break
                length = self.ctypes.c_uint16.from_address(
                    entry + self.Dirent.name_length.offset
                ).value
                name = self.ctypes.string_at(entry + self.Dirent.name.offset, length)
                if name not in {b".", b".."}:
                    result.append(_os.fsdecode(name))
        finally:
            self.rewinddir(directory)
            self.closedir(directory)
        return result

    def mkdir(self, path, mode=0o777, *, dir_fd=None):
        if dir_fd is None or _os.mkdir in _os.supports_dir_fd:
            return _os.mkdir(path, mode, dir_fd=dir_fd)
        self.checked(self.mkdirat(dir_fd, self._path_bytes(path), mode), path)

    def readlink(self, path, *, dir_fd=None):
        if dir_fd is None or _os.readlink in _os.supports_dir_fd:
            return _os.readlink(path, dir_fd=dir_fd)
        capacity = 256
        while True:
            buffer = self.ctypes.create_string_buffer(capacity)
            length = self.checked(
                self.readlinkat(dir_fd, self._path_bytes(path), buffer, capacity), path
            )
            if length < capacity:
                value = buffer.raw[:length]
                return (
                    value
                    if isinstance(_os.fspath(path), bytes)
                    else _os.fsdecode(value)
                )
            capacity *= 2

    def unlink(self, path, *, dir_fd=None):
        if dir_fd is None or _os.unlink in _os.supports_dir_fd:
            return _os.unlink(path, dir_fd=dir_fd)
        self.checked(self.unlinkat(dir_fd, self._path_bytes(path), 0), path)

    def rmdir(self, path, *, dir_fd=None):
        if dir_fd is None or _os.rmdir in _os.supports_dir_fd:
            return _os.rmdir(path, dir_fd=dir_fd)
        self.checked(self.unlinkat(dir_fd, self._path_bytes(path), 0x80), path)

    def rename(self, source, destination, *, src_dir_fd=None, dst_dir_fd=None):
        if _os.rename in _os.supports_dir_fd or src_dir_fd is dst_dir_fd is None:
            return _os.rename(
                source, destination, src_dir_fd=src_dir_fd, dst_dir_fd=dst_dir_fd
            )
        self.checked(
            self.renameat(
                self.descriptor(src_dir_fd),
                self._path_bytes(source),
                self.descriptor(dst_dir_fd),
                self._path_bytes(destination),
            ),
            source,
        )

    def link(
        self,
        source,
        destination,
        *,
        src_dir_fd=None,
        dst_dir_fd=None,
        follow_symlinks=True,
    ):
        if _os.link in _os.supports_dir_fd or src_dir_fd is dst_dir_fd is None:
            return _os.link(
                source,
                destination,
                src_dir_fd=src_dir_fd,
                dst_dir_fd=dst_dir_fd,
                follow_symlinks=follow_symlinks,
            )
        self.checked(
            self.linkat(
                self.descriptor(src_dir_fd),
                self._path_bytes(source),
                self.descriptor(dst_dir_fd),
                self._path_bytes(destination),
                0x40 if follow_symlinks else 0,
            ),
            source,
        )

    def symlink(self, source, destination, target_is_directory=False, *, dir_fd=None):
        if dir_fd is None or _os.symlink in _os.supports_dir_fd:
            return _os.symlink(
                source,
                destination,
                target_is_directory=target_is_directory,
                dir_fd=dir_fd,
            )
        self.checked(
            self.symlinkat(
                self._path_bytes(source), dir_fd, self._path_bytes(destination)
            ),
            destination,
        )

    def utime(self, path, *, ns, dir_fd=None, follow_symlinks=True):
        if dir_fd is None or _os.utime in _os.supports_dir_fd:
            return _os.utime(
                path, ns=ns, dir_fd=dir_fd, follow_symlinks=follow_symlinks
            )
        times = (self.Timespec * 2)(
            *[self.Timespec(*divmod(value, 1000000000)) for value in ns]
        )
        self.checked(
            self.utimensat(
                dir_fd, self._path_bytes(path), times, 0 if follow_symlinks else 0x20
            ),
            path,
        )


os = (
    DarwinOS()
    if sys.platform == "darwin"
    and (
        _os.listdir not in _os.supports_fd
        or any(
            function not in _os.supports_dir_fd
            for function in (
                _os.open,
                _os.stat,
                _os.mkdir,
                _os.readlink,
                _os.rename,
                _os.unlink,
                _os.rmdir,
                _os.link,
                _os.symlink,
                _os.utime,
            )
        )
    )
    else _os
)
