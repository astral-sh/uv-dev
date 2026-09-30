import importlib.util
import io
import random
import tarfile
import tempfile
import unittest
import zlib
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "sdist_metadata_oracle", Path(__file__).with_name("sdist_metadata_oracle.py")
)
assert spec is not None and spec.loader is not None
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)


class SourcePrefixTests(unittest.TestCase):
    def test_prefix_reaches_metadata_without_reading_the_trailing_payload(self):
        metadata = b"Metadata-Version: 2.4\nName: example\nVersion: 1.0\n\n"
        randomizer = random.Random(42)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "example.tar.gz"
            with tarfile.open(path, "w:gz", format=tarfile.USTAR_FORMAT) as archive:
                for name, body in (
                    ("example-1.0/before", randomizer.randbytes(100000)),
                    ("example-1.0/PKG-INFO", metadata),
                    ("example-1.0/after", randomizer.randbytes(100000)),
                ):
                    entry = tarfile.TarInfo(name)
                    entry.size = len(body)
                    archive.addfile(entry, io.BytesIO(body))
            prefix, offset, actual = oracle.metadata_prefix(path)
            self.assertEqual(actual, metadata)
            self.assertLess(len(prefix), path.stat().st_size - 90000)
            decoder = zlib.decompressobj(wbits=31)
            decoded = decoder.decompress(prefix)
            self.assertEqual(decoded[offset : offset + len(metadata)], metadata)
            self.assertFalse(decoder.eof)


if __name__ == "__main__":
    unittest.main()
