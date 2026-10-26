import importlib.util
import io
import tarfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "validate_archive", Path(__file__).with_name("validate-archive.py")
)
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


def archive(*members):
    result = io.BytesIO()
    with tarfile.open(fileobj=result, mode="w") as tar:
        for member in members:
            tar.addfile(
                member, io.BytesIO(b"x" * member.size) if member.isfile() else None
            )
    result.seek(0)
    return result


class ArchiveTests(unittest.TestCase):
    roots = ("../../../.cargo/registry", "target")

    def test_portable_members(self):
        file = tarfile.TarInfo("target/debug/example")
        file.size = 1
        symlink = tarfile.TarInfo("target/debug/link")
        symlink.type = tarfile.SYMTYPE
        symlink.linkname = "example"
        validator.validate(archive(file, symlink), self.roots)

    def test_reject_members_outside_cache_roots(self):
        for name in [
            ".git/config",
            "target/../../.ssh/config",
            "/etc/passwd",
            "target\\..\\other",
        ]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                validator.validate(archive(tarfile.TarInfo(name)), self.roots)

    def test_reject_nonportable_links(self):
        for kind in [tarfile.SYMTYPE, tarfile.LNKTYPE]:
            member = tarfile.TarInfo("target/link")
            member.type = kind
            member.linkname = "/home/runner/work/uv/uv/target/file"
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                validator.validate(archive(member), self.roots)

    def test_reject_devices(self):
        member = tarfile.TarInfo("target/device")
        member.type = tarfile.CHRTYPE
        with self.assertRaises(ValueError):
            validator.validate(archive(member), self.roots)


if __name__ == "__main__":
    unittest.main()
