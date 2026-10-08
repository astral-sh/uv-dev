"""Exercise the generated documentation handoff without publication."""

import hashlib
import importlib.util
import io
import json
import tarfile
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "docs_artifact", Path(__file__).with_name("docs-artifact.py")
)
assert SPEC is not None and SPEC.loader is not None
ARTIFACT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ARTIFACT)
COMMIT = "a" * 40
VERSION = "0.12.23"


class DocsArtifact(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.site = self.root / "generated"
        self.site.mkdir()
        (self.site / "assets").mkdir()
        (self.site / "index.html").write_bytes(b"<p>Docs</p>")
        (self.site / "assets/font.woff").write_bytes(b"\x00\xff\x01")
        (self.site / ".nojekyll").write_bytes(b"")
        self.archive = self.root / "publication.tar"
        self.output = self.root / "restored"

    def pack(self):
        ARTIFACT.pack(self.site, self.archive, COMMIT, VERSION)

    def unpack(self, commit=COMMIT, version=VERSION):
        ARTIFACT.unpack(self.archive, self.output, commit, version)

    def rewrite(self, update):
        self.pack()
        with tarfile.open(self.archive) as archive:
            entries = [
                (member, archive.extractfile(member).read())
                for member in archive.getmembers()
            ]
        entries = update(entries)
        with tarfile.open(self.archive, "w") as archive:
            for member, data in entries:
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))

    def test_roundtrip_preserves_complete_site(self):
        self.pack()
        self.unpack()
        expected = {
            path.relative_to(self.site).as_posix(): path.read_bytes()
            for path in self.site.rglob("*")
            if path.is_file()
        }
        actual = {
            path.relative_to(self.output).as_posix(): path.read_bytes()
            for path in self.output.rglob("*")
            if path.is_file()
        }
        self.assertEqual(actual, expected)

    def test_wrong_source_or_version_is_rejected(self):
        self.pack()
        for commit, version in (("b" * 40, VERSION), (COMMIT, "other")):
            with (
                self.subTest(commit=commit, version=version),
                self.assertRaises(ValueError),
            ):
                self.unpack(commit, version)
        self.assertFalse(self.output.exists())

    def test_changed_contents_are_rejected_before_publication(self):
        self.rewrite(
            lambda entries: [
                (member, b"changed" if member.name.endswith("index.html") else data)
                for member, data in entries
            ]
        )
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            self.unpack()
        self.assertFalse(self.output.exists())
        self.assertEqual(list(self.root.glob(".docs-*")), [])

    def test_unexpected_files_and_duplicate_entries_are_rejected(self):
        for kind in ("extra", "duplicate"):
            with self.subTest(kind=kind):
                if self.archive.exists():
                    self.archive.unlink()

                def extra(entries, kind=kind):
                    if kind == "duplicate":
                        return [*entries, entries[0]]
                    return [*entries, (tarfile.TarInfo("unexpected"), b"extra")]

                self.rewrite(extra)
                diagnostic = (
                    "duplicate documentation archive entries"
                    if kind == "duplicate"
                    else "differs from its file inventory"
                )
                with self.assertRaisesRegex(ValueError, diagnostic):
                    self.unpack()
                self.assertFalse(self.output.exists())

    def test_link_entries_are_rejected(self):
        def link(entries):
            member = tarfile.TarInfo("site/link")
            member.type = tarfile.SYMTYPE
            member.linkname = "../outside"
            return [*entries, (member, b"")]

        self.rewrite(link)
        with self.assertRaisesRegex(ValueError, "regular files"):
            self.unpack()

    def test_noncanonical_manifest_paths_are_rejected(self):
        for name in (
            "../outside",
            "/outside",
            "assets/../outside",
            ".git/config",
            "assets\\outside",
            "./index.html",
        ):
            with self.subTest(name=name):
                manifest = {
                    "schema": 1,
                    "source_commit": COMMIT,
                    "version": VERSION,
                    "files": {name: hashlib.sha256(b"file").hexdigest()},
                }
                with tarfile.open(self.archive, "w") as archive:
                    ARTIFACT.add_file(
                        archive, "manifest.json", json.dumps(manifest).encode()
                    )
                    ARTIFACT.add_file(archive, "site/" + name, b"file")
                with self.assertRaises(ValueError):
                    self.unpack()
                self.assertFalse(self.output.exists())

    def test_producer_rejects_symlinks(self):
        (self.site / "link").symlink_to(self.site / "index.html")
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.pack()

    def test_existing_destination_is_untouched(self):
        self.pack()
        self.output.mkdir()
        (self.output / "existing").write_text("keep")
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.unpack()
        self.assertEqual((self.output / "existing").read_text(), "keep")


if __name__ == "__main__":
    unittest.main()
