"""Host filesystem and WASI support for the generated Rust program."""

import errno
import io
import struct

from ._posix import os
from ._wasi_impl import Preview1


class Host:
    def __init__(self, arguments, version="0.13.0", source_tree=None):
        arguments = [os.fsdecode(argument) for argument in arguments]
        if any("\0" in argument for argument in arguments):
            raise ValueError("embedded null byte")
        self.wasi = Preview1(
            ["uv-build", *arguments],
            {"/": "/"},
            {
                **os.environ,
                "UV_BUILD_SLIM_CWD": os.getcwd(),
                "UV_BUILD_SLIM_VERSION": version,
                "UV_BUILD_SLIM_SOURCE_TREE": source_tree
                if source_tree is not None
                else os.getcwd(),
            },
        )
        self.stdout = io.BytesIO()
        self.stderr = io.BytesIO()
        self.wasi.fd[1].pipe = self.stdout
        self.wasi.fd[2].pipe = self.stderr
        self.wasi.fd[1].wasm_type = self.wasi.FILETYPE_UNKNOWN
        self.wasi.fd[2].wasm_type = self.wasi.FILETYPE_UNKNOWN
        self.native_errors = {}
        for name in dir(self.wasi):
            if name.startswith("ERRNO_"):
                constant = "E" + name[len("ERRNO_") :]
                if hasattr(errno, constant):
                    self.native_errors[getattr(self.wasi, name)] = getattr(
                        errno, constant
                    )
        self.native_errors[0] = 0

    def __call__(self, module, name, context, arguments):
        # Wasm integers carry bits; Preview 1 pointers, descriptors, lengths, and
        # rights are unsigned. Only fd_seek's displacement is signed.
        if module == "wasi_snapshot_preview1":
            wide = {
                "clock_time_get": {1},
                "fd_readdir": {3},
                "fd_seek": {1},
                "path_open": {5, 6},
            }.get(name, set())
            arguments = [
                value
                if name == "fd_seek" and index == 1
                else value & ((1 << (64 if index in wide else 32)) - 1)
                for index, value in enumerate(arguments)
            ]
        elif module == "uv_build_slim" and name in {
            "file_mode",
            "native_error_message",
        }:
            arguments = [
                value
                if name == "native_error_message" and index == 0
                else value & 0xFFFFFFFF
                for index, value in enumerate(arguments)
            ]
        if module == "uv_build_slim" and name == "native_errno":
            return [self.native_errors.get(arguments[0], arguments[0])]
        if module == "uv_build_slim" and name == "native_error_message":
            code, offset, pointer, capacity = arguments
            native = self.native_errors.get(code, code)
            message = (
                os.strerror(native)
                .encode("utf-8", "surrogateescape")
                .decode("utf-8", "replace")
                .encode("utf-8")
            )
            chunk = (
                message[offset : offset + capacity]
                .decode("utf-8", "ignore")
                .encode("utf-8")
            )
            context.store.mems[0].put(pointer, chunk)
            return [len(chunk)]
        if module == "uv_build_slim" and name == "file_mode":
            try:
                path = os.fsdecode(
                    bytes(context.store.mems[0].get(arguments[0], arguments[1]))
                )
                return [os.stat(path).st_mode if os.name != "nt" else 0o644]
            except OSError as error:
                return [0x80000000 | self.error_number(error)]
        if module != "wasi_snapshot_preview1":
            raise RuntimeError("Unknown host module: " + module)
        if name == "random_get":
            context.store.mems[0].put(arguments[0], os.urandom(arguments[1]))
            return [0]
        try:
            if name in {
                "args_get",
                "args_sizes_get",
                "environ_get",
                "environ_sizes_get",
            }:
                values = self.wasi.args if name.startswith("args_") else self.wasi.envs
                return self.strings(
                    context, arguments, values, name.endswith("sizes_get")
                )
            if name == "fd_filestat_get":
                return self.filestat(context, arguments)
            if name == "path_filestat_get":
                return self.pathstat(context, arguments)
            if name == "fd_seek":
                return self.seek(context, arguments)
            if name == "fd_readdir":
                return self.readdir(context, arguments)
            if name == "path_readlink":
                return self.readlink(context, arguments)
            return getattr(self.wasi, name)(context, arguments)
        except OSError as error:
            return [self.error_number(error)]

    def error_number(self, error):
        code = errno.errorcode.get(error.errno, "EIO")
        code = {"EOPNOTSUPP": "ENOTSUP", "EWOULDBLOCK": "EAGAIN"}.get(code, code)
        return getattr(self.wasi, "ERRNO_" + code[1:], self.wasi.ERRNO_IO)

    def strings(self, context, arguments, values, sizes):
        encoded = [value.encode("utf-8", "surrogateescape") + b"\0" for value in values]
        memory = context.store.mems[0]
        if sizes:
            memory.put_u32(arguments[0], len(encoded))
            memory.put_u32(arguments[1], sum(map(len, encoded)))
        else:
            pointer, buffer = arguments
            for value in encoded:
                memory.put_u32(pointer, buffer)
                memory.put(buffer, value)
                pointer += 4
                buffer += len(value)
        return [0]

    def write_stat(self, memory, pointer, info):
        memory.put_u64(pointer, info.st_dev)
        memory.put_u64(pointer + 8, info.st_ino)
        memory.put_u8(pointer + 16, self.wasi.help_wasm_type(info))
        for offset, value in [
            (24, info.st_nlink),
            (32, info.st_size),
            (40, info.st_atime_ns),
            (48, info.st_mtime_ns),
            (56, info.st_ctime_ns),
        ]:
            memory.put_u64(pointer + offset, value)

    def filestat(self, context, arguments):
        descriptor, output = arguments
        if self.wasi.help_badf(descriptor):
            return [self.wasi.ERRNO_BADF]
        if self.wasi.help_perm(descriptor, self.wasi.RIGHTS_FD_FILESTAT_GET):
            return [self.wasi.ERRNO_NOTCAPABLE]
        info = os.fstat(self.wasi.fd[descriptor].host_fd)
        self.write_stat(context.store.mems[0], output, info)
        return [0]

    def pathstat(self, context, arguments):
        descriptor, flags, pointer, length, output = arguments
        if self.wasi.help_badf(descriptor):
            return [self.wasi.ERRNO_BADF]
        if self.wasi.help_perm(descriptor, self.wasi.RIGHTS_PATH_FILESTAT_GET):
            return [self.wasi.ERRNO_NOTCAPABLE]
        memory = context.store.mems[0]
        path = bytes(memory.get(pointer, length))
        info = os.stat(
            path,
            dir_fd=self.wasi.fd[descriptor].host_fd,
            follow_symlinks=bool(flags & self.wasi.LOOKUPFLAGS_SYMLINK_FOLLOW),
        )
        self.write_stat(memory, output, info)
        return [0]

    def seek(self, context, arguments):
        descriptor, offset, whence, output = arguments
        if self.wasi.help_badf(descriptor):
            return [self.wasi.ERRNO_BADF]
        if self.wasi.help_idir(descriptor):
            return [self.wasi.ERRNO_ISDIR]
        if self.wasi.help_perm(descriptor, self.wasi.RIGHTS_FD_SEEK):
            return [self.wasi.ERRNO_NOTCAPABLE]
        position = os.lseek(self.wasi.fd[descriptor].host_fd, offset, whence)
        context.store.mems[0].put_u64(output, position)
        return [0]

    def readdir(self, context, arguments):
        descriptor, buffer, capacity, cookie, used = arguments
        if self.wasi.help_badf(descriptor):
            return [self.wasi.ERRNO_BADF]
        if self.wasi.help_perm(descriptor, self.wasi.RIGHTS_FD_READDIR):
            return [self.wasi.ERRNO_NOTCAPABLE]
        directory = self.wasi.fd[descriptor]
        names = [b".", b"..", *sorted(map(os.fsencode, os.listdir(directory.host_fd)))]
        result = bytearray()
        for index in range(cookie, len(names)):
            name = names[index]
            info = os.stat(name, dir_fd=directory.host_fd, follow_symlinks=False)
            entry = (
                struct.pack(
                    "<QQIB3x",
                    index + 1,
                    info.st_ino,
                    len(name),
                    self.wasi.help_wasm_type(info),
                )
                + name
            )
            available = capacity - len(result)
            result.extend(entry[:available])
            if len(entry) >= available:
                break
        memory = context.store.mems[0]
        memory.put(buffer, result)
        memory.put_u32(used, len(result))
        return [0]

    def readlink(self, context, arguments):
        descriptor, pointer, length, output, capacity, used = arguments
        if self.wasi.help_badf(descriptor):
            return [self.wasi.ERRNO_BADF]
        if self.wasi.help_perm(descriptor, self.wasi.RIGHTS_PATH_READLINK):
            return [self.wasi.ERRNO_NOTCAPABLE]
        memory = context.store.mems[0]
        name = bytes(memory.get(pointer, length))
        directory = self.wasi.fd[descriptor]
        data = os.readlink(name, dir_fd=directory.host_fd)
        data = os.fsencode(data)[:capacity]
        memory.put(output, data)
        memory.put_u32(used, len(data))
        return [0]

    def close(self):
        for entry in self.wasi.fd[3:]:
            if entry.host_status == self.wasi.FILE_STATUS_OPENED:
                try:
                    os.close(entry.host_fd)
                except OSError:
                    pass
