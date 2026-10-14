"""Native filesystem semantics at the WASI host boundary."""

import os
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from uv_build_slim_wasm._host import Host
from uv_build_slim_wasm._posix import DarwinOS
from uv_build_slim_wasm._posix import os as filesystem
from uv_build_slim_wasm._vm import Memory


class DescriptorCompatibility(unittest.TestCase):
    def test_facade_retains_os_path(self):
        facade = DarwinOS.__new__(DarwinOS)
        self.assertIs(facade.path, os.path)

    def test_directory_descriptor_survives_rename(self):
        scratch = Path(
            os.environ.get("TMPDIR", Path.home() / "code/tmp/uv-build-slim-poc/tmp")
        )
        with tempfile.TemporaryDirectory(dir=str(scratch)) as temporary:
            directory = Path(temporary) / "directory"
            directory.mkdir()
            (directory / "entry").write_bytes(b"abc")
            expected = (directory / "entry").stat()
            descriptor = os.open(directory, os.O_RDONLY)
            try:
                directory.rename(directory.with_name("renamed"))
                self.assertEqual(filesystem.listdir(descriptor), ["entry"])
                actual = filesystem.stat("entry", dir_fd=descriptor)
                self.assertEqual(
                    (actual.st_ino, actual.st_size), (expected.st_ino, expected.st_size)
                )
                file = filesystem.open("entry", os.O_RDONLY, dir_fd=descriptor)
                try:
                    self.assertEqual(os.read(file, 3), b"abc")
                finally:
                    os.close(file)
            finally:
                os.close(descriptor)


class HostParity(unittest.TestCase):
    def setUp(self):
        scratch = Path(
            os.environ.get("TMPDIR", Path.home() / "code/tmp/uv-build-slim-poc/tmp")
        )
        self.temporary = tempfile.TemporaryDirectory(dir=str(scratch))
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / "file"
        self.path.write_bytes(b"abc")
        self.host = Host([])
        self.addCleanup(self.host.close)
        self.memory = Memory(1, 1)
        self.context = SimpleNamespace(
            store=SimpleNamespace(mems=[self.memory]),
            stack=SimpleNamespace(
                frame=[SimpleNamespace(module=SimpleNamespace(mems=[0]))]
            ),
        )
        name = os.fsencode(self.path)
        self.memory.put(1024, name)
        self.length = len(name)
        wasi = self.host.wasi
        rights = wasi.RIGHTS_FD_READ | wasi.RIGHTS_FD_SEEK | wasi.RIGHTS_FD_FILESTAT_GET
        self.assertEqual(
            self.call(
                "path_open",
                [
                    3,
                    wasi.LOOKUPFLAGS_SYMLINK_FOLLOW,
                    1024,
                    len(name),
                    0,
                    rights,
                    0,
                    0,
                    2048,
                ],
            ),
            [0],
        )
        self.descriptor = self.memory.get_u32(2048)

    def call(self, name, arguments):
        return self.host("wasi_snapshot_preview1", name, self.context, arguments)

    def test_open_descriptor_survives_unlink(self):
        expected = os.stat(self.path)
        self.path.unlink()
        self.assertEqual(self.call("fd_filestat_get", [self.descriptor, 4096]), [0])
        self.assertEqual(self.memory.get_u64(4096), expected.st_dev)
        self.assertEqual(self.memory.get_u64(4104), expected.st_ino)
        self.assertEqual(self.memory.get_u64(4128), expected.st_size)

    def test_path_metadata_retains_device_and_inode(self):
        expected = os.stat(self.path)
        self.assertEqual(
            self.call(
                "path_filestat_get",
                [3, self.host.wasi.LOOKUPFLAGS_SYMLINK_FOLLOW, 1024, self.length, 4096],
            ),
            [0],
        )
        self.assertEqual(self.memory.get_u64(4096), expected.st_dev)
        self.assertEqual(self.memory.get_u64(4104), expected.st_ino)

    def test_failed_seek_does_not_move_position(self):
        native = self.host.wasi.fd[self.descriptor].host_fd
        os.lseek(native, 1, os.SEEK_SET)
        self.assertEqual(
            self.call("fd_seek", [self.descriptor, -5, os.SEEK_END, 4096]),
            [self.host.wasi.ERRNO_INVAL],
        )
        self.assertEqual(os.lseek(native, 0, os.SEEK_CUR), 1)

    def test_invalid_unsigned_descriptor_does_not_alias_last_entry(self):
        self.assertEqual(self.call("fd_close", [-1]), [self.host.wasi.ERRNO_BADF])
        self.assertEqual(self.call("fd_filestat_get", [self.descriptor, 4096]), [0])


if __name__ == "__main__":
    unittest.main()
