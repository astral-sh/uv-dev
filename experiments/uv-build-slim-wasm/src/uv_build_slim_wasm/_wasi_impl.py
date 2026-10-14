# Derived from libraries/pywasm at 47abba6c94ecf74c3c173344684891f2ca29c2e3.
# MIT license; see LICENSE-PYWASM.
from __future__ import annotations
import dataclasses
import fcntl
import io
from ._posix import os
import platform
import random
import select
import socket
import stat
import sys
import time

class Preview1:
    ADVICE_NORMAL = 0
    ADVICE_SEQUENTIAL = 1
    ADVICE_RANDOM = 2
    ADVICE_WILLNEED = 3
    ADVICE_DONTNEED = 4
    ADVICE_NOREUSE = 5
    CLOCKID_REALTIME = 0
    CLOCKID_MONOTONIC = 1
    CLOCKID_PROCESS_CPUTIME_ID = 2
    CLOCKID_THREAD_CPUTIME_ID = 3
    DIRCOOKIE_START = 0
    ERRNO_SUCCESS = 0
    ERRNO_2BIG = 1
    ERRNO_ACCES = 2
    ERRNO_ADDRINUSE = 3
    ERRNO_ADDRNOTAVAIL = 4
    ERRNO_AFNOSUPPORT = 5
    ERRNO_AGAIN = 6
    ERRNO_ALREADY = 7
    ERRNO_BADF = 8
    ERRNO_BADMSG = 9
    ERRNO_BUSY = 10
    ERRNO_CANCELED = 11
    ERRNO_CHILD = 12
    ERRNO_CONNABORTED = 13
    ERRNO_CONNREFUSED = 14
    ERRNO_CONNRESET = 15
    ERRNO_DEADLK = 16
    ERRNO_DESTADDRREQ = 17
    ERRNO_DOM = 18
    ERRNO_DQUOT = 19
    ERRNO_EXIST = 20
    ERRNO_FAULT = 21
    ERRNO_FBIG = 22
    ERRNO_HOSTUNREACH = 23
    ERRNO_IDRM = 24
    ERRNO_ILSEQ = 25
    ERRNO_INPROGRESS = 26
    ERRNO_INTR = 27
    ERRNO_INVAL = 28
    ERRNO_IO = 29
    ERRNO_ISCONN = 30
    ERRNO_ISDIR = 31
    ERRNO_LOOP = 32
    ERRNO_MFILE = 33
    ERRNO_MLINK = 34
    ERRNO_MSGSIZE = 35
    ERRNO_MULTIHOP = 36
    ERRNO_NAMETOOLONG = 37
    ERRNO_NETDOWN = 38
    ERRNO_NETRESET = 39
    ERRNO_NETUNREACH = 40
    ERRNO_NFILE = 41
    ERRNO_NOBUFS = 42
    ERRNO_NODEV = 43
    ERRNO_NOENT = 44
    ERRNO_NOEXEC = 45
    ERRNO_NOLCK = 46
    ERRNO_NOLINK = 47
    ERRNO_NOMEM = 48
    ERRNO_NOMSG = 49
    ERRNO_NOPROTOOPT = 50
    ERRNO_NOSPC = 51
    ERRNO_NOSYS = 52
    ERRNO_NOTCONN = 53
    ERRNO_NOTDIR = 54
    ERRNO_NOTEMPTY = 55
    ERRNO_NOTRECOVERABLE = 56
    ERRNO_NOTSOCK = 57
    ERRNO_NOTSUP = 58
    ERRNO_NOTTY = 59
    ERRNO_NXIO = 60
    ERRNO_OVERFLOW = 61
    ERRNO_OWNERDEAD = 62
    ERRNO_PERM = 63
    ERRNO_PIPE = 64
    ERRNO_PROTO = 65
    ERRNO_PROTONOSUPPORT = 66
    ERRNO_PROTOTYPE = 67
    ERRNO_RANGE = 68
    ERRNO_ROFS = 69
    ERRNO_SPIPE = 70
    ERRNO_SRCH = 71
    ERRNO_STALE = 72
    ERRNO_TIMEDOUT = 73
    ERRNO_TXTBSY = 74
    ERRNO_XDEV = 75
    ERRNO_NOTCAPABLE = 76
    EVENTRWFLAGS_FD_READWRITE_HANGUP = 1 << 0
    EVENTTYPE_CLOCK = 0
    EVENTTYPE_FD_READ = 1
    EVENTTYPE_FD_WRITE = 2
    FDFLAGS_APPEND = 1 << 0
    FDFLAGS_DSYNC = 1 << 1
    FDFLAGS_NONBLOCK = 1 << 2
    FDFLAGS_RSYNC = 1 << 3
    FDFLAGS_SYNC = 1 << 4
    FD_STDIN = 0
    FD_STDOUT = 1
    FD_STDERR = 2
    FILETYPE_UNKNOWN = 0
    FILETYPE_BLOCK_DEVICE = 1
    FILETYPE_CHARACTER_DEVICE = 2
    FILETYPE_DIRECTORY = 3
    FILETYPE_REGULAR_FILE = 4
    FILETYPE_SOCKET_DGRAM = 5
    FILETYPE_SOCKET_STREAM = 6
    FILETYPE_SYMBOLIC_LINK = 7
    FSTFLAGS_ATIM = 1 << 0
    FSTFLAGS_ATIM_NOW = 1 << 1
    FSTFLAGS_MTIM = 1 << 2
    FSTFLAGS_MTIM_NOW = 1 << 3
    LOOKUPFLAGS_SYMLINK_FOLLOW = 1 << 0
    OFLAGS_CREAT = 1 << 0
    OFLAGS_DIRECTORY = 1 << 1
    OFLAGS_EXCL = 1 << 2
    OFLAGS_TRUNC = 1 << 3
    PREOPENTYPE_DIR = 0
    RIFLAGS_RECV_PEEK = 1 << 0
    RIFLAGS_RECV_WAITALL = 1 << 1
    RIGHTS_FD_DATASYNC = 1 << 0
    RIGHTS_FD_READ = 1 << 1
    RIGHTS_FD_SEEK = 1 << 2
    RIGHTS_FD_FDSTAT_SET_FLAGS = 1 << 3
    RIGHTS_FD_SYNC = 1 << 4
    RIGHTS_FD_TELL = 1 << 5
    RIGHTS_FD_WRITE = 1 << 6
    RIGHTS_FD_ADVISE = 1 << 7
    RIGHTS_FD_ALLOCATE = 1 << 8
    RIGHTS_PATH_CREATE_DIRECTORY = 1 << 9
    RIGHTS_PATH_CREATE_FILE = 1 << 10
    RIGHTS_PATH_LINK_SOURCE = 1 << 11
    RIGHTS_PATH_LINK_TARGET = 1 << 12
    RIGHTS_PATH_OPEN = 1 << 13
    RIGHTS_FD_READDIR = 1 << 14
    RIGHTS_PATH_READLINK = 1 << 15
    RIGHTS_PATH_RENAME_SOURCE = 1 << 16
    RIGHTS_PATH_RENAME_TARGET = 1 << 17
    RIGHTS_PATH_FILESTAT_GET = 1 << 18
    RIGHTS_PATH_FILESTAT_SET_SIZE = 1 << 19
    RIGHTS_PATH_FILESTAT_SET_TIMES = 1 << 20
    RIGHTS_FD_FILESTAT_GET = 1 << 21
    RIGHTS_FD_FILESTAT_SET_SIZE = 1 << 22
    RIGHTS_FD_FILESTAT_SET_TIMES = 1 << 23
    RIGHTS_PATH_SYMLINK = 1 << 24
    RIGHTS_PATH_REMOVE_DIRECTORY = 1 << 25
    RIGHTS_PATH_UNLINK_FILE = 1 << 26
    RIGHTS_POLL_FD_READWRITE = 1 << 27
    RIGHTS_SOCK_SHUTDOWN = 1 << 28
    RIGHTS_SOCK_ACCEPT = 1 << 29
    ROFLAGS_RECV_DATA_TRUNCATED = 1 << 0
    SDFLAGS_RD = 1 << 0
    SDFLAGS_WR = 1 << 1
    SDFLAGS_RW = SDFLAGS_RD | SDFLAGS_WR
    SIGNAL_NONE = 0
    SIGNAL_HUP = 1
    SIGNAL_INT = 2
    SIGNAL_QUIT = 3
    SIGNAL_ILL = 4
    SIGNAL_TRAP = 5
    SIGNAL_ABRT = 6
    SIGNAL_BUS = 7
    SIGNAL_FPE = 8
    SIGNAL_KILL = 9
    SIGNAL_USR1 = 10
    SIGNAL_SEGV = 11
    SIGNAL_USR2 = 12
    SIGNAL_PIPE = 13
    SIGNAL_ALRM = 14
    SIGNAL_TERM = 15
    SIGNAL_CHLD = 16
    SIGNAL_CONT = 17
    SIGNAL_STOP = 18
    SIGNAL_TSTP = 19
    SIGNAL_TTIN = 20
    SIGNAL_TTOU = 21
    SIGNAL_URG = 22
    SIGNAL_XCPU = 23
    SIGNAL_XFSZ = 24
    SIGNAL_VTALRM = 25
    SIGNAL_PROF = 26
    SIGNAL_WINCH = 27
    SIGNAL_POLL = 28
    SIGNAL_PWR = 29
    SIGNAL_SYS = 30
    SUBCLOCKFLAGS_SUBSCRIPTION_CLOCK_ABSTIME = 1 << 0
    WHENCE_SET = 0
    WHENCE_CUR = 1
    WHENCE_END = 2
    FILE_STATUS_OPENED = 0
    FILE_STATUS_CLOSED = 1

    @dataclasses.dataclass
    class File:
        host_fd: int
        host_flag: int
        host_name: str
        host_status: int
        pipe: io.IOBase | None
        wasm_fd: int
        wasm_flag: int
        wasm_name: str
        wasm_rights_base: int
        wasm_rights_root: int
        wasm_status: int
        wasm_type: int

    def __init__(self, args: list[str], dirs: dict[str, str], envs: dict[str, str]):
        self.args = args
        self.dirs = {os.path.normpath(k): os.path.normpath(v) for k, v in dirs.items()}
        self.envs = ['='.join(e) for e in sorted(envs.items())]
        self.fd: list[Preview1.File] = []
        self.fd.append(self.File(host_fd=0, host_flag=0, host_name='<stdin>', host_status=self.FILE_STATUS_OPENED, pipe=None, wasm_fd=self.FD_STDIN, wasm_flag=0, wasm_name='<stdin>', wasm_rights_base=self.RIGHTS_POLL_FD_READWRITE | self.RIGHTS_FD_READ, wasm_rights_root=0, wasm_status=self.FILE_STATUS_OPENED, wasm_type=self.FILETYPE_CHARACTER_DEVICE))
        self.fd.append(self.File(host_fd=1, host_flag=0, host_name='<stdout>', host_status=self.FILE_STATUS_OPENED, pipe=None, wasm_fd=self.FD_STDOUT, wasm_flag=0, wasm_name='<stdout>', wasm_rights_base=self.RIGHTS_POLL_FD_READWRITE | self.RIGHTS_FD_WRITE, wasm_rights_root=0, wasm_status=self.FILE_STATUS_OPENED, wasm_type=self.FILETYPE_CHARACTER_DEVICE))
        self.fd.append(self.File(host_fd=2, host_flag=0, host_name='<stderr>', host_status=self.FILE_STATUS_OPENED, pipe=None, wasm_fd=self.FD_STDERR, wasm_flag=0, wasm_name='<stderr>', wasm_rights_base=self.RIGHTS_POLL_FD_READWRITE | self.RIGHTS_FD_WRITE, wasm_rights_root=0, wasm_status=self.FILE_STATUS_OPENED, wasm_type=self.FILETYPE_CHARACTER_DEVICE))
        for k, v in sorted(self.dirs.items()):
            k = os.path.normpath(k)
            v = os.path.abspath(os.path.normpath(v))
            self.fd.append(self.File(host_fd=os.open(v, os.O_RDONLY | os.O_DIRECTORY), host_flag=os.O_RDONLY | os.O_DIRECTORY, host_name=v, host_status=self.FILE_STATUS_OPENED, pipe=None, wasm_fd=len(self.fd), wasm_flag=0, wasm_name=k, wasm_rights_base=130022912, wasm_rights_root=268435455, wasm_status=self.FILE_STATUS_OPENED, wasm_type=self.FILETYPE_DIRECTORY))
        self.return_on_exit = 1

    def args_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        argv = args[0]
        argv_buf = args[1]
        for e in self.args:
            mems.put_u32(argv, argv_buf)
            argv += 4
            mems.put(argv_buf, bytearray(e.encode(errors='surrogateescape')))
            argv_buf += len(e)
            mems.put_u8(argv_buf, 0)
            argv_buf += 1
        return [self.ERRNO_SUCCESS]

    def args_sizes_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        mems.put_u32(args[0], len(self.args))
        mems.put_u32(args[1], sum([len(e) + 1 for e in self.args]))
        return [self.ERRNO_SUCCESS]

    def clock_res_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        mems.put_u64(args[1], 1)
        return [self.ERRNO_SUCCESS]

    def clock_time_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        mems.put_u64(args[2], self.help_time(args[0]))
        return [self.ERRNO_SUCCESS]

    def environ_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        environ = args[0]
        environ_buf = args[1]
        for e in self.envs:
            mems.put_u32(environ, environ_buf)
            environ += 4
            mems.put(environ_buf, bytearray(e.encode(errors='surrogateescape')))
            environ_buf += len(e)
            mems.put_u8(environ_buf, 0)
            environ_buf += 1
        return [self.ERRNO_SUCCESS]

    def environ_sizes_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        mems.put_u32(args[0], len(self.envs))
        mems.put_u32(args[1], sum([len(e) + 1 for e in self.envs]))
        return [self.ERRNO_SUCCESS]

    def fd_advise(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_FD_ADVISE):
            return [self.ERRNO_NOTCAPABLE]
        return [self.ERRNO_SUCCESS]

    def fd_allocate(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_idir(args[0]):
            return [self.ERRNO_ISDIR]
        if self.help_perm(args[0], self.RIGHTS_FD_ALLOCATE):
            return [self.ERRNO_NOTCAPABLE]
        file = self.fd[args[0]]
        size = args[1] + args[2] - os.stat(file.host_fd).st_size
        if size <= 0:
            return [self.ERRNO_SUCCESS]
        offs = os.lseek(file.host_fd, 0, os.SEEK_CUR)
        os.lseek(file.host_fd, 0, os.SEEK_END)
        os.write(file.host_fd, bytearray(size))
        os.lseek(file.host_fd, offs, os.SEEK_SET)
        return [self.ERRNO_SUCCESS]

    def fd_close(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        file = self.fd[args[0]]
        if file.wasm_type != self.FILETYPE_CHARACTER_DEVICE:
            os.close(file.host_fd)
            file.host_status = self.FILE_STATUS_CLOSED
        file.wasm_status = self.FILE_STATUS_CLOSED
        return [self.ERRNO_SUCCESS]

    def fd_datasync(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_FD_DATASYNC):
            return [self.ERRNO_NOTCAPABLE]
        return [self.ERRNO_SUCCESS]

    def fd_fdstat_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        mems.put_u8(args[1], file.wasm_type)
        mems.put_u16(args[1] + 2, file.wasm_flag)
        mems.put_u64(args[1] + 8, file.wasm_rights_base)
        mems.put_u64(args[1] + 16, file.wasm_rights_root)
        return [self.ERRNO_SUCCESS]

    def fd_fdstat_set_flags(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_FD_FDSTAT_SET_FLAGS):
            return [self.ERRNO_NOTCAPABLE]
        file = self.fd[args[0]]
        fyes = self.FDFLAGS_APPEND | self.FDFLAGS_APPEND
        if file.wasm_flag ^ args[1] | fyes != fyes:
            return [self.ERRNO_INVAL]
        file.host_flag &= ~os.O_APPEND
        file.host_flag &= ~os.O_NONBLOCK
        file.wasm_flag &= ~self.FDFLAGS_APPEND
        file.wasm_flag &= ~self.FDFLAGS_NONBLOCK
        if args[1] & self.FDFLAGS_APPEND:
            file.host_flag |= os.O_APPEND
            file.wasm_flag |= self.FDFLAGS_APPEND
        if args[1] & self.FDFLAGS_NONBLOCK:
            file.host_flag |= os.O_NONBLOCK
            file.wasm_flag |= self.FDFLAGS_NONBLOCK
        fcntl.fcntl(file.host_fd, fcntl.F_SETFL, file.host_flag)
        return [self.ERRNO_SUCCESS]

    def fd_fdstat_set_rights(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        file = self.fd[args[0]]
        if file.wasm_rights_base & args[1] != args[1]:
            return [self.ERRNO_NOTCAPABLE]
        if file.wasm_rights_root & args[2] != args[2]:
            return [self.ERRNO_NOTCAPABLE]
        file.wasm_rights_base = args[1]
        file.wasm_rights_root = args[2]
        return [self.ERRNO_SUCCESS]

    def fd_filestat_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_FD_FILESTAT_GET):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        info = os.stat(file.host_name)
        mems.put_u64(args[1], 1)
        mems.put_u64(args[1] + 8, info.st_ino)
        mems.put_u8(args[1] + 16, file.wasm_type)
        mems.put_u64(args[1] + 24, info.st_nlink)
        mems.put_u64(args[1] + 32, info.st_size)
        mems.put_u64(args[1] + 40, info.st_atime_ns)
        mems.put_u64(args[1] + 48, info.st_mtime_ns)
        mems.put_u64(args[1] + 56, info.st_ctime_ns)
        return [self.ERRNO_SUCCESS]

    def fd_filestat_set_size(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_idir(args[0]):
            return [self.ERRNO_ISDIR]
        if self.help_perm(args[0], self.RIGHTS_FD_FILESTAT_SET_SIZE):
            return [self.ERRNO_NOTCAPABLE]
        file = self.fd[args[0]]
        os.ftruncate(file.host_fd, args[1])
        return [self.ERRNO_SUCCESS]

    def fd_filestat_set_times(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_FD_FILESTAT_SET_TIMES):
            return [self.ERRNO_NOTCAPABLE]
        if args[3] & self.FSTFLAGS_ATIM and args[3] & self.FSTFLAGS_ATIM_NOW:
            return [self.ERRNO_INVAL]
        if args[3] & self.FSTFLAGS_MTIM and args[3] & self.FSTFLAGS_MTIM_NOW:
            return [self.ERRNO_INVAL]
        file = self.fd[args[0]]
        info = os.stat(file.host_fd)
        atim = info.st_atime_ns
        if args[3] & self.FSTFLAGS_ATIM:
            atim = args[1]
        if args[3] & self.FSTFLAGS_ATIM_NOW:
            atim = time.time_ns()
        mtim = info.st_mtime_ns
        if args[3] & self.FSTFLAGS_MTIM:
            mtim = args[2]
        if args[3] & self.FSTFLAGS_MTIM_NOW:
            mtim = args[2]
        os.utime(file.host_fd, ns=(atim, mtim))
        return [self.ERRNO_SUCCESS]

    def fd_pread(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_idir(args[0]):
            return [self.ERRNO_ISDIR]
        if self.help_perm(args[0], self.RIGHTS_FD_READ):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        iovs_ptr = args[1]
        iovs_len = args[2]
        offs = args[3]
        for _ in range(iovs_len):
            elem_ptr = mems.get_u32(iovs_ptr)
            elem_len = mems.get_u32(iovs_ptr + 4)
            data = os.pread(file.host_fd, elem_len, offs)
            if len(data) == 0:
                break
            mems.put(elem_ptr, bytearray(data))
            iovs_ptr += 8
            offs += len(data)
            if len(data) < elem_len:
                break
        mems.put_u32(args[4], offs - args[3])
        return [self.ERRNO_SUCCESS]

    def fd_prestat_dir_name(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]) or args[0] - 2 > len(self.dirs):
            return [self.ERRNO_BADF]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        name = bytearray(file.wasm_name.encode(errors='surrogateescape'))
        if len(name) > args[2]:
            return [self.ERRNO_NAMETOOLONG]
        mems.put(args[1], name)
        return [self.ERRNO_SUCCESS]

    def fd_prestat_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]) or args[0] - 2 > len(self.dirs):
            return [self.ERRNO_BADF]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        name = bytearray(file.wasm_name.encode(errors='surrogateescape'))
        mems.put_u8(args[1], self.PREOPENTYPE_DIR)
        mems.put_u32(args[1] + 4, len(name))
        return [self.ERRNO_SUCCESS]

    def fd_pwrite(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_idir(args[0]):
            return [self.ERRNO_ISDIR]
        if self.help_perm(args[0], self.RIGHTS_FD_WRITE):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        iovs_ptr = args[1]
        iovs_len = args[2]
        data = bytearray()
        for _ in range(iovs_len):
            elem_ptr = mems.get_u32(iovs_ptr)
            elem_len = mems.get_u32(iovs_ptr + 4)
            elem = mems.get(elem_ptr, elem_len)
            iovs_ptr += 8
            data.extend(elem)
        size = os.pwrite(file.host_fd, data, args[3])
        mems.put_u32(args[4], size)
        return [self.ERRNO_SUCCESS]

    def fd_read(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_idir(args[0]):
            return [self.ERRNO_ISDIR]
        if self.help_perm(args[0], self.RIGHTS_FD_READ):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        iovs_ptr = args[1]
        iovs_len = args[2]
        size = 0
        for _ in range(iovs_len):
            elem_ptr = mems.get_u32(iovs_ptr)
            elem_len = mems.get_u32(iovs_ptr + 4)
            data = [lambda: os.read(file.host_fd, elem_len), lambda: file.pipe.read(elem_len) if file.pipe else b''][self.help_pipe(args[0])]()
            if len(data) == 0:
                break
            mems.put(elem_ptr, bytearray(data))
            iovs_ptr += 8
            size += len(data)
            if len(data) < elem_len:
                break
        mems.put_u32(args[3], size)
        return [self.ERRNO_SUCCESS]

    def fd_readdir(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_FD_READDIR):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        dirent = args[1]
        buflen = args[2]
        bufend = dirent + buflen
        cookie = args[3]
        result = ['.', '..', *sorted(os.listdir(file.host_fd))]
        for name in result[cookie:]:
            if dirent + 24 > bufend:
                break
            info = os.stat(os.path.normpath(os.path.join(file.host_name, name)))
            mems.put_u64(dirent, cookie + 1)
            mems.put_u64(dirent + 8, info.st_ino)
            mems.put_u32(dirent + 16, len(name))
            mems.put_u8(dirent + 20, self.help_wasm_type(info))
            dirent += 24
            if dirent + len(name) > bufend:
                break
            mems.put(dirent, bytearray(name.encode(errors='surrogateescape')))
            dirent += len(name)
            cookie += 1
        mems.put_u32(args[4], dirent - args[1] if cookie >= len(result) else buflen)
        return [self.ERRNO_SUCCESS]

    def fd_renumber(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]) or self.help_badf(args[1]):
            return [self.ERRNO_BADF]
        stem = self.fd[args[0]]
        dest = self.fd[args[1]]
        if dest.wasm_type != self.FILETYPE_CHARACTER_DEVICE:
            os.close(dest.host_fd)
        dest.host_fd = stem.host_fd
        dest.host_flag = stem.host_flag
        dest.host_name = stem.host_name
        dest.host_status = stem.host_status
        dest.pipe = stem.pipe
        dest.wasm_fd = stem.wasm_fd
        dest.wasm_flag = stem.wasm_flag
        dest.wasm_name = stem.wasm_name
        dest.wasm_rights_base = stem.wasm_rights_base
        dest.wasm_rights_root = stem.wasm_rights_root
        dest.wasm_status = stem.wasm_status
        dest.wasm_type = stem.wasm_type
        stem.host_status = self.FILE_STATUS_CLOSED
        stem.wasm_status = self.FILE_STATUS_CLOSED
        return [self.ERRNO_SUCCESS]

    def fd_seek(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_idir(args[0]):
            return [self.ERRNO_ISDIR]
        if self.help_perm(args[0], self.RIGHTS_FD_SEEK):
            return [self.ERRNO_NOTCAPABLE]
        assert self.WHENCE_SET == os.SEEK_SET
        assert self.WHENCE_CUR == os.SEEK_CUR
        assert self.WHENCE_END == os.SEEK_END
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        ocur = os.lseek(file.host_fd, 0, args[2])
        if ocur + args[1] < 0:
            return [self.ERRNO_INVAL]
        offs = os.lseek(file.host_fd, args[1], args[2])
        mems.put_u64(args[3], offs)
        return [self.ERRNO_SUCCESS]

    def fd_sync(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_idir(args[0]):
            return [self.ERRNO_ISDIR]
        if self.help_perm(args[0], self.RIGHTS_FD_SYNC):
            return [self.ERRNO_NOTCAPABLE]
        file = self.fd[args[0]]
        os.fsync(file.host_fd)
        return [self.ERRNO_SUCCESS]

    def fd_tell(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_idir(args[0]):
            return [self.ERRNO_ISDIR]
        if self.help_perm(args[0], self.RIGHTS_FD_TELL):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        offs = os.lseek(file.host_fd, 0, os.SEEK_CUR)
        mems.put_u64(args[1], offs)
        return [self.ERRNO_SUCCESS]

    def fd_write(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_idir(args[0]):
            return [self.ERRNO_ISDIR]
        if self.help_perm(args[0], self.RIGHTS_FD_WRITE):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        iovs_ptr = args[1]
        iovs_len = args[2]
        data = bytearray()
        for _ in range(iovs_len):
            elem_ptr = mems.get_u32(iovs_ptr)
            elem_len = mems.get_u32(iovs_ptr + 4)
            elem = mems.get(elem_ptr, elem_len)
            iovs_ptr += 8
            data.extend(elem)
        size = [lambda: os.write(file.host_fd, data), lambda: file.pipe.write(data) if file.pipe else 0][int(self.help_pipe(args[0]))]()
        mems.put_u32(args[3], size)
        return [self.ERRNO_SUCCESS]

    def help_badf(self, fd: int) -> bool:
        return fd < 0 or fd >= len(self.fd) or self.fd[fd].wasm_status != self.FILE_STATUS_OPENED

    def help_escp(self, root: str, name: str) -> bool:
        return not os.path.normpath(os.path.join(root, name)).startswith(root)

    def help_wasm_type(self, info: os.stat_result) -> int:
        _matched_value_1 = info.st_mode
        if True and stat.S_ISBLK(_matched_value_1):
            x = _matched_value_1
            return self.FILETYPE_BLOCK_DEVICE
        elif True and stat.S_ISCHR(_matched_value_1):
            x = _matched_value_1
            return self.FILETYPE_CHARACTER_DEVICE
        elif True and stat.S_ISDIR(_matched_value_1):
            x = _matched_value_1
            return self.FILETYPE_DIRECTORY
        elif True and stat.S_ISREG(_matched_value_1):
            x = _matched_value_1
            return self.FILETYPE_REGULAR_FILE
        elif True and stat.S_ISSOCK(_matched_value_1):
            x = _matched_value_1
            return self.FILETYPE_UNKNOWN
        elif True and stat.S_ISLNK(_matched_value_1):
            x = _matched_value_1
            return self.FILETYPE_SYMBOLIC_LINK
        elif True:
            return self.FILETYPE_UNKNOWN

    def help_idir(self, fd: int) -> bool:
        return self.fd[fd].wasm_type == self.FILETYPE_DIRECTORY

    def help_perm(self, fd: int, perm: int) -> bool:
        return self.fd[fd].wasm_rights_base & perm == 0

    def help_pipe(self, fd: int) -> bool:
        return fd < 3 and self.fd[fd].pipe is not None

    def help_sock(self, fd: int) -> bool:
        return self.fd[fd].wasm_type not in [self.FILETYPE_SOCKET_DGRAM, self.FILETYPE_SOCKET_STREAM]

    def help_time(self, id: int) -> int:
        _matched_value_2 = id
        if _matched_value_2 == self.CLOCKID_REALTIME:
            return time.time_ns()
        elif _matched_value_2 == self.CLOCKID_MONOTONIC:
            return time.monotonic_ns()
        elif _matched_value_2 == self.CLOCKID_PROCESS_CPUTIME_ID:
            return time.process_time_ns()
        elif _matched_value_2 == self.CLOCKID_THREAD_CPUTIME_ID:
            return time.thread_time_ns()
        elif True:
            return 0

    def path_create_directory(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_PATH_CREATE_DIRECTORY):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        name = mems.get(args[1], args[2]).decode(errors='surrogateescape')
        if self.help_escp(file.wasm_name, name):
            return [self.ERRNO_PERM]
        try:
            os.mkdir(name, dir_fd=file.host_fd)
        except FileExistsError:
            return [self.ERRNO_EXIST]
        return [self.ERRNO_SUCCESS]

    def path_filestat_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_PATH_FILESTAT_GET):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        flag = args[1]
        name = mems.get(args[2], args[3]).decode(errors='surrogateescape')
        if self.help_escp(file.wasm_name, name):
            return [self.ERRNO_PERM]
        foll = flag & self.LOOKUPFLAGS_SYMLINK_FOLLOW != 0
        try:
            info = os.stat(name, dir_fd=file.host_fd, follow_symlinks=foll)
        except FileNotFoundError:
            return [self.ERRNO_NOENT]
        mems.put_u64(args[4], 1)
        mems.put_u64(args[4] + 8, info.st_ino)
        mems.put_u8(args[4] + 16, self.help_wasm_type(info))
        mems.put_u64(args[4] + 24, info.st_nlink)
        mems.put_u64(args[4] + 32, info.st_size)
        mems.put_u64(args[4] + 40, info.st_atime_ns)
        mems.put_u64(args[4] + 48, info.st_mtime_ns)
        mems.put_u64(args[4] + 56, info.st_ctime_ns)
        return [self.ERRNO_SUCCESS]

    def path_filestat_set_times(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_PATH_FILESTAT_SET_TIMES):
            return [self.ERRNO_NOTCAPABLE]
        if args[6] & self.FSTFLAGS_ATIM and args[6] & self.FSTFLAGS_ATIM_NOW:
            return [self.ERRNO_INVAL]
        if args[6] & self.FSTFLAGS_MTIM and args[6] & self.FSTFLAGS_MTIM_NOW:
            return [self.ERRNO_INVAL]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        flag = args[1]
        name = mems.get(args[2], args[3]).decode(errors='surrogateescape')
        if self.help_escp(file.wasm_name, name):
            return [self.ERRNO_PERM]
        info = os.stat(name, dir_fd=file.host_fd)
        atim = info.st_atime_ns
        if args[6] & self.FSTFLAGS_ATIM:
            atim = args[4]
        if args[6] & self.FSTFLAGS_ATIM_NOW:
            atim = time.time_ns()
        mtim = info.st_mtime_ns
        if args[6] & self.FSTFLAGS_MTIM:
            mtim = args[5]
        if args[6] & self.FSTFLAGS_MTIM_NOW:
            mtim = time.time_ns()
        foll = flag & self.LOOKUPFLAGS_SYMLINK_FOLLOW != 0
        try:
            os.utime(name, ns=(atim, mtim), dir_fd=file.host_fd, follow_symlinks=foll)
        except FileNotFoundError:
            return [self.ERRNO_NOENT]
        return [self.ERRNO_SUCCESS]

    def path_link(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]) or self.help_badf(args[4]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_PATH_LINK_SOURCE):
            return [self.ERRNO_NOTCAPABLE]
        if self.help_perm(args[4], self.RIGHTS_PATH_LINK_TARGET):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        stem = self.fd[args[0]]
        dest = self.fd[args[4]]
        stem_name = mems.get(args[2], args[3]).decode(errors='surrogateescape')
        dest_name = mems.get(args[5], args[6]).decode(errors='surrogateescape')
        if self.help_escp(stem.wasm_name, stem_name):
            return [self.ERRNO_PERM]
        if self.help_escp(dest.wasm_name, dest_name):
            return [self.ERRNO_PERM]
        foll = args[1] & self.LOOKUPFLAGS_SYMLINK_FOLLOW != 0
        try:
            os.link(stem_name, dest_name, src_dir_fd=stem.host_fd, dst_dir_fd=dest.host_fd, follow_symlinks=foll)
        except FileExistsError:
            return [self.ERRNO_EXIST]
        except FileNotFoundError:
            return [self.ERRNO_NOENT]
        except PermissionError:
            return [self.ERRNO_PERM]
        return [self.ERRNO_SUCCESS]

    def path_open(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_PATH_OPEN):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        name = mems.get(args[2], args[3]).decode(errors='surrogateescape')
        wasm_name = os.path.normpath(os.path.join(file.wasm_name, name))
        host_name = os.path.normpath(os.path.join(file.host_name, name))
        if '\x00' in name:
            return [self.ERRNO_ILSEQ]
        if self.help_escp(file.host_name, name):
            return [self.ERRNO_PERM]
        flag = 0
        if args[1] & self.LOOKUPFLAGS_SYMLINK_FOLLOW == 0:
            flag |= os.O_NOFOLLOW
        if args[4] & self.OFLAGS_CREAT:
            flag |= os.O_CREAT
        if args[4] & self.OFLAGS_DIRECTORY:
            flag |= os.O_DIRECTORY
        if args[4] & self.OFLAGS_EXCL:
            flag |= os.O_EXCL
        if args[4] & self.OFLAGS_TRUNC:
            flag |= os.O_TRUNC
            if file.wasm_rights_base & self.RIGHTS_PATH_FILESTAT_SET_SIZE == 0:
                return [self.ERRNO_PERM]
        if args[5] & self.RIGHTS_FD_READ + self.RIGHTS_FD_WRITE == self.RIGHTS_FD_READ:
            flag |= os.O_RDONLY
        if args[5] & self.RIGHTS_FD_READ + self.RIGHTS_FD_WRITE == self.RIGHTS_FD_WRITE:
            flag |= os.O_WRONLY
        if args[5] & self.RIGHTS_FD_READ + self.RIGHTS_FD_WRITE == self.RIGHTS_FD_READ + self.RIGHTS_FD_WRITE:
            flag |= os.O_RDWR
        if args[7] & self.FDFLAGS_APPEND:
            flag |= os.O_APPEND
        wasm_fd = len(self.fd)
        try:
            host_fd = os.open(name, flag, 438, dir_fd=self.fd[args[0]].host_fd)
        except FileExistsError:
            return [self.ERRNO_EXIST]
        except FileNotFoundError:
            return [self.ERRNO_NOENT]
        except IsADirectoryError:
            return [self.ERRNO_ISDIR]
        except NotADirectoryError:
            return [self.ERRNO_NOTDIR]
        except OSError as e:
            if platform.system().lower() == 'darwin' and e.errno == 62:
                return [self.ERRNO_LOOP]
            if platform.system().lower() == 'linux' and e.errno == 40:
                return [self.ERRNO_LOOP]
            raise e
        rights_base = args[5]
        rights_root = args[6]
        if os.path.isdir(host_fd):
            void = self.RIGHTS_FD_SEEK
            rights_base &= ~void
            rights_root &= ~void
        if os.path.isfile(host_fd):
            void = sum([self.RIGHTS_PATH_CREATE_DIRECTORY, self.RIGHTS_PATH_CREATE_FILE, self.RIGHTS_PATH_LINK_SOURCE, self.RIGHTS_PATH_LINK_TARGET, self.RIGHTS_PATH_OPEN, self.RIGHTS_PATH_READLINK, self.RIGHTS_PATH_RENAME_SOURCE, self.RIGHTS_PATH_RENAME_TARGET, self.RIGHTS_PATH_FILESTAT_GET, self.RIGHTS_PATH_FILESTAT_SET_SIZE, self.RIGHTS_PATH_FILESTAT_SET_TIMES, self.RIGHTS_PATH_SYMLINK, self.RIGHTS_PATH_REMOVE_DIRECTORY, self.RIGHTS_PATH_UNLINK_FILE])
            rights_base &= ~void
            rights_root &= ~void
        self.fd.append(Preview1.File(host_fd=host_fd, host_flag=flag, host_name=host_name, host_status=self.FILE_STATUS_OPENED, pipe=None, wasm_fd=wasm_fd, wasm_flag=args[7], wasm_name=wasm_name, wasm_rights_base=rights_base, wasm_rights_root=rights_root, wasm_status=self.FILE_STATUS_OPENED, wasm_type=self.help_wasm_type(os.fstat(host_fd))))
        mems.put_u32(args[8], wasm_fd)
        return [self.ERRNO_SUCCESS]

    def path_readlink(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_PATH_READLINK):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        name = mems.get(args[1], args[2]).decode(errors='surrogateescape')
        if self.help_escp(file.wasm_name, name):
            return [self.ERRNO_PERM]
        try:
            data = os.readlink(name, dir_fd=file.host_fd)
        except FileNotFoundError:
            return [self.ERRNO_NOENT]
        size = min(len(data), args[4])
        mems.put(args[3], bytearray(data[:size].encode(errors='surrogateescape')))
        mems.put_u32(args[5], size)
        return [self.ERRNO_SUCCESS]

    def path_remove_directory(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_PATH_REMOVE_DIRECTORY):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        name = mems.get(args[1], args[2]).decode(errors='surrogateescape')
        if self.help_escp(file.wasm_name, name):
            return [self.ERRNO_PERM]
        try:
            os.rmdir(name, dir_fd=file.host_fd)
        except NotADirectoryError:
            return [self.ERRNO_NOTDIR]
        except OSError as e:
            if platform.system().lower() == 'darwin' and e.errno == 66:
                return [self.ERRNO_NOTEMPTY]
            if platform.system().lower() == 'linux' and e.errno == 39:
                return [self.ERRNO_NOTEMPTY]
            raise e
        return [self.ERRNO_SUCCESS]

    def path_rename(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]) or self.help_badf(args[3]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_PATH_RENAME_SOURCE):
            return [self.ERRNO_NOTCAPABLE]
        if self.help_perm(args[3], self.RIGHTS_PATH_RENAME_TARGET):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        stem = self.fd[args[0]]
        dest = self.fd[args[3]]
        stem_name = mems.get(args[1], args[2]).decode(errors='surrogateescape')
        dest_name = mems.get(args[4], args[5]).decode(errors='surrogateescape')
        if self.help_escp(stem.wasm_name, stem_name):
            return [self.ERRNO_PERM]
        if self.help_escp(dest.wasm_name, dest_name):
            return [self.ERRNO_PERM]
        try:
            os.rename(stem_name, dest_name, src_dir_fd=stem.host_fd, dst_dir_fd=dest.host_fd)
        except FileExistsError:
            return [self.ERRNO_EXIST]
        except FileNotFoundError:
            return [self.ERRNO_NOENT]
        except IsADirectoryError:
            return [self.ERRNO_ISDIR]
        except NotADirectoryError:
            return [self.ERRNO_NOTDIR]
        except OSError as e:
            if platform.system().lower() == 'darwin' and e.errno == 66:
                return [self.ERRNO_NOTEMPTY]
            if platform.system().lower() == 'linux' and e.errno == 39:
                return [self.ERRNO_NOTEMPTY]
            raise e
        return [self.ERRNO_SUCCESS]

    def path_symlink(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[2]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[2], self.RIGHTS_PATH_SYMLINK):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[2]]
        stem = mems.get(args[0], args[1]).decode(errors='surrogateescape')
        dest = mems.get(args[3], args[4]).decode(errors='surrogateescape')
        if self.help_escp(file.wasm_name, stem):
            return [self.ERRNO_PERM]
        if self.help_escp(file.wasm_name, dest):
            return [self.ERRNO_PERM]
        try:
            os.symlink(stem, dest, dir_fd=file.host_fd)
        except FileExistsError:
            return [self.ERRNO_EXIST]
        except FileNotFoundError:
            return [self.ERRNO_NOENT]
        except NotADirectoryError:
            return [self.ERRNO_NOTDIR]
        return [self.ERRNO_SUCCESS]

    def path_unlink_file(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_perm(args[0], self.RIGHTS_PATH_UNLINK_FILE):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        name = mems.get(args[1], args[2]).decode(errors='surrogateescape')
        if self.help_escp(file.wasm_name, name):
            return [self.ERRNO_PERM]
        try:
            os.unlink(name, dir_fd=file.host_fd)
        except IsADirectoryError:
            return [self.ERRNO_ISDIR]
        except NotADirectoryError:
            return [self.ERRNO_NOTDIR]
        except PermissionError:
            return [self.ERRNO_PERM]
        return [self.ERRNO_SUCCESS]

    def poll_oneoff(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        subs = args[0]
        outs = args[1]
        nsub = args[2]
        nout = args[3]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        csub = []
        rsub = []
        wsub = []
        for i in range(nsub):
            addr = subs + i * 48
            userdata = mems.get_u64(addr)
            eype = mems.get_u8(addr + 8)
            _matched_value_3 = eype
            if _matched_value_3 == self.EVENTTYPE_CLOCK:
                ckid = mems.get_u64(addr + 16)
                nval = mems.get_u64(addr + 24)
                flag = mems.get_u32(addr + 40)
                ctim = self.help_time(ckid)
                delt = ctim - nval if flag & self.SUBCLOCKFLAGS_SUBSCRIPTION_CLOCK_ABSTIME else nval
                ends = nval if flag & self.SUBCLOCKFLAGS_SUBSCRIPTION_CLOCK_ABSTIME else ctim + nval
                csub.append([userdata, ckid, delt, ends])
            elif _matched_value_3 == self.EVENTTYPE_FD_READ:
                file = mems.get_u32(addr + 16)
                if self.help_badf(file):
                    return [self.ERRNO_BADF]
                if self.help_perm(file, self.RIGHTS_POLL_FD_READWRITE):
                    return [self.ERRNO_NOTCAPABLE]
                rsub.append([userdata, file])
            elif _matched_value_3 == self.EVENTTYPE_FD_WRITE:
                file = mems.get_u32(addr + 16)
                if self.help_badf(file):
                    return [self.ERRNO_BADF]
                if self.help_perm(file, self.RIGHTS_POLL_FD_READWRITE):
                    return [self.ERRNO_NOTCAPABLE]
                wsub.append([userdata, file])
        delt = 8
        if csub:
            delt = min([e[2] for e in csub]) / 1000000000.0
        ryes, wyes, _ = select.select([self.fd[fd].host_fd for _, fd in rsub], [self.fd[fd].host_fd for _, fd in wsub], [], delt)
        neve = 0
        for e in csub:
            if self.help_time(e[1]) < e[3]:
                continue
            mems.put_u64(outs, e[0])
            mems.put_u16(outs + 8, self.ERRNO_SUCCESS)
            mems.put_u8(outs + 10, self.EVENTTYPE_CLOCK)
            outs += 32
            neve += 1
        for e in rsub:
            if self.fd[e[1]].host_fd not in ryes:
                continue
            mems.put_u64(outs, e[0])
            mems.put_u16(outs + 8, self.ERRNO_SUCCESS)
            mems.put_u8(outs + 10, self.EVENTTYPE_FD_READ)
            outs += 32
            neve += 1
        for e in wsub:
            if self.fd[e[1]].host_fd not in wyes:
                continue
            mems.put_u64(outs, e[0])
            mems.put_u16(outs + 8, self.ERRNO_SUCCESS)
            mems.put_u8(outs + 10, self.EVENTTYPE_FD_WRITE)
            outs += 32
            neve += 1
        mems.put_u32(nout, neve)
        return [self.ERRNO_SUCCESS]

    def proc_exit(self, _: pywasm.core.Machine, args: list[int]) -> None:
        if self.return_on_exit:
            raise Exception(SystemExit, args[0])
        sys.exit(args[0])

    def proc_raise(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if args[0] != self.SIGNAL_NONE:
            return [self.ERRNO_INVAL]
        return [self.ERRNO_SUCCESS]

    def random_get(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        mems.put(args[0], bytearray(random.randbytes(args[1])))
        return [self.ERRNO_SUCCESS]

    def sched_yield(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        assert len(args) == 0
        return [self.ERRNO_SUCCESS]

    def sock_accept(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_sock(args[0]):
            return [self.ERRNO_NOTSOCK]
        if self.help_perm(args[0], self.RIGHTS_SOCK_ACCEPT):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        kype = {self.FILETYPE_SOCKET_STREAM: socket.SOCK_STREAM, self.FILETYPE_SOCKET_DGRAM: socket.SOCK_DGRAM}[file.wasm_type]
        sock = socket.fromfd(file.host_fd, socket.AF_INET, kype)
        if args[1] & self.FDFLAGS_NONBLOCK:
            sock.setblocking(False)
        conn, _ = sock.accept()
        host_fd = conn.fileno()
        wasm_fd = len(self.fd)
        self.fd.append(self.File(host_fd=host_fd, host_flag=0, host_name=file.host_name, host_status=self.FILE_STATUS_OPENED, pipe=None, wasm_fd=wasm_fd, wasm_flag=args[1], wasm_name=file.wasm_name, wasm_rights_base=file.wasm_rights_base, wasm_rights_root=file.wasm_rights_root, wasm_status=self.FILE_STATUS_OPENED, wasm_type=file.wasm_type))
        mems.put_u32(args[2], wasm_fd)
        return [self.ERRNO_SUCCESS]

    def sock_recv(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_sock(args[0]):
            return [self.ERRNO_NOTSOCK]
        if self.help_perm(args[0], self.RIGHTS_FD_READ):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        kype = {self.FILETYPE_SOCKET_STREAM: socket.SOCK_STREAM, self.FILETYPE_SOCKET_DGRAM: socket.SOCK_DGRAM}[file.wasm_type]
        sock = socket.fromfd(file.host_fd, socket.AF_INET, kype)
        iovs_ptr = args[1]
        iovs_len = args[2]
        ro_flags = 0
        ri_flags = 0
        if args[3] & self.RIFLAGS_RECV_PEEK:
            ri_flags |= socket.MSG_PEEK
        if args[3] & self.RIFLAGS_RECV_WAITALL and file.wasm_type == self.FILETYPE_SOCKET_STREAM:
            ri_flags |= socket.MSG_WAITALL
        size_all = 0
        for _ in range(iovs_len):
            elem_len = mems.get_u32(iovs_ptr + 4)
            size_all += elem_len
            iovs_ptr += 8
        try:
            data = sock.recv(size_all, ri_flags)
        except BrokenPipeError:
            return [self.ERRNO_PIPE]
        data_len = len(data)
        if data_len == 0 and file.wasm_type == self.FILETYPE_SOCKET_STREAM:
            ro_flags |= self.EVENTRWFLAGS_FD_READWRITE_HANGUP
        iovs_ptr = args[1]
        offs = 0
        for _ in range(iovs_len):
            if offs >= data_len:
                break
            elem_ptr = mems.get_u32(iovs_ptr)
            elem_len = mems.get_u32(iovs_ptr + 4)
            size = min(elem_len, data_len - offs)
            if size > 0:
                mems.put(elem_ptr, bytearray(data[offs:offs + size]))
                offs += size
            iovs_ptr += 8
        if data_len < size_all:
            ro_flags |= self.ROFLAGS_RECV_DATA_TRUNCATED
        mems.put_u32(args[4], data_len)
        mems.put_u32(args[5], ro_flags)
        return [self.ERRNO_SUCCESS]

    def sock_send(self, m: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_sock(args[0]):
            return [self.ERRNO_NOTSOCK]
        if self.help_perm(args[0], self.RIGHTS_FD_WRITE):
            return [self.ERRNO_NOTCAPABLE]
        mems = m.store.mems[m.stack.frame[-1].module.mems[0]]
        file = self.fd[args[0]]
        kype = {self.FILETYPE_SOCKET_STREAM: socket.SOCK_STREAM, self.FILETYPE_SOCKET_DGRAM: socket.SOCK_DGRAM}[file.wasm_type]
        sock = socket.fromfd(file.host_fd, socket.AF_INET, kype)
        iovs_ptr = args[1]
        iovs_len = args[2]
        data = bytearray()
        for _ in range(iovs_len):
            elem_ptr = mems.get_u32(iovs_ptr)
            elem_len = mems.get_u32(iovs_ptr + 4)
            elem = mems.get(elem_ptr, elem_len)
            data.extend(elem)
            iovs_ptr += 8
        try:
            size = sock.send(data)
        except BrokenPipeError:
            return [self.ERRNO_PIPE]
        mems.put_u32(args[4], size)
        return [self.ERRNO_SUCCESS]

    def sock_shutdown(self, _: pywasm.core.Machine, args: list[int]) -> list[int]:
        if self.help_badf(args[0]):
            return [self.ERRNO_BADF]
        if self.help_sock(args[0]):
            return [self.ERRNO_NOTSOCK]
        if self.help_perm(args[0], self.RIGHTS_SOCK_SHUTDOWN):
            return [self.ERRNO_NOTCAPABLE]
        file = self.fd[args[0]]
        kype = {self.FILETYPE_SOCKET_STREAM: socket.SOCK_STREAM, self.FILETYPE_SOCKET_DGRAM: socket.SOCK_DGRAM}[file.wasm_type]
        sock = socket.fromfd(file.host_fd, socket.AF_INET, kype)
        shut = 0
        _matched_value_4 = args[1]
        if _matched_value_4 == self.SDFLAGS_RD:
            shut = socket.SHUT_RD
        elif _matched_value_4 == self.SDFLAGS_WR:
            shut = socket.SHUT_WR
        elif _matched_value_4 == self.SDFLAGS_RW:
            shut = socket.SHUT_RDWR
        elif True:
            return [self.ERRNO_INVAL]
        try:
            sock.shutdown(shut)
        except OSError as e:
            if platform.system().lower() == 'darwin' and e.errno == 57:
                return [self.ERRNO_CONNABORTED]
            raise e
        return [self.ERRNO_SUCCESS]
