"""Compare rendered Win32 console text and attributes with native Rust."""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


def probe(root, reference):
    import ctypes
    import shutil

    sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
    import uv_build_slim_wasm as backend

    word, dword = ctypes.c_uint16, ctypes.c_uint32
    handle, boolean = ctypes.c_void_p, ctypes.c_int

    class Coordinate(ctypes.Structure):
        _fields_ = [("x", ctypes.c_int16), ("y", ctypes.c_int16)]

    class Rectangle(ctypes.Structure):
        _fields_ = [
            ("left", ctypes.c_int16),
            ("top", ctypes.c_int16),
            ("right", ctypes.c_int16),
            ("bottom", ctypes.c_int16),
        ]

    class Information(ctypes.Structure):
        _fields_ = [
            ("size", Coordinate),
            ("cursor", Coordinate),
            ("attributes", word),
            ("window", Rectangle),
            ("maximum", Coordinate),
        ]

    class Character(ctypes.Union):
        _fields_ = [("unicode", ctypes.c_wchar), ("ascii", ctypes.c_char)]

    class Cell(ctypes.Structure):
        _fields_ = [("character", Character), ("attributes", word)]

    assert ctypes.sizeof(Information) == 22 and ctypes.sizeof(Cell) == 4
    library = ctypes.WinDLL("kernel32", use_last_error=True)

    def function(name, result, arguments):
        value = getattr(library, name)
        value.restype, value.argtypes = result, arguments
        return value

    allocate = function("AllocConsole", boolean, [])
    release = function("FreeConsole", boolean, [])
    standard = function("GetStdHandle", handle, [dword])
    set_standard = function("SetStdHandle", boolean, [dword, handle])
    create_file = function(
        "CreateFileW",
        handle,
        [ctypes.c_wchar_p, dword, dword, handle, dword, dword, handle],
    )
    close_handle = function("CloseHandle", boolean, [handle])
    information = function(
        "GetConsoleScreenBufferInfo", boolean, [handle, ctypes.POINTER(Information)]
    )
    get_mode = function("GetConsoleMode", boolean, [handle, ctypes.POINTER(dword)])
    set_mode = function("SetConsoleMode", boolean, [handle, dword])
    set_attribute = function("SetConsoleTextAttribute", boolean, [handle, word])
    set_cursor = function("SetConsoleCursorPosition", boolean, [handle, Coordinate])
    fill_text = function(
        "FillConsoleOutputCharacterW",
        boolean,
        [handle, ctypes.c_wchar, dword, Coordinate, ctypes.POINTER(dword)],
    )
    fill_attributes = function(
        "FillConsoleOutputAttribute",
        boolean,
        [handle, word, dword, Coordinate, ctypes.POINTER(dword)],
    )
    read = function(
        "ReadConsoleOutputW",
        boolean,
        [
            handle,
            ctypes.POINTER(Cell),
            Coordinate,
            Coordinate,
            ctypes.POINTER(Rectangle),
        ],
    )

    def checked(value):
        if not value:
            raise ctypes.WinError(ctypes.get_last_error())

    release()
    checked(allocate())
    console = None
    try:
        # STARTF_USESTDHANDLES keeps inherited pipes after AllocConsole. Open the
        # new screen buffer explicitly before passing stderr to either backend.
        console = create_file("CONOUT$", 0xC0000000, 3, None, 3, 0, None)
        if console in {None, ctypes.c_void_p(-1).value}:
            raise ctypes.WinError(ctypes.get_last_error())
        checked(set_standard(0xFFFFFFF4, console))
        assert standard(0xFFFFFFF4) == console
        initial, mode = Information(), dword()
        checked(information(console, ctypes.byref(initial)))
        checked(get_mode(console, ctypes.byref(mode)))

        def clear():
            written = dword()
            count = initial.size.x * initial.size.y
            checked(set_mode(console, mode.value))
            checked(set_attribute(console, initial.attributes))
            checked(
                fill_text(console, " ", count, Coordinate(0, 0), ctypes.byref(written))
            )
            checked(
                fill_attributes(
                    console,
                    initial.attributes,
                    count,
                    Coordinate(0, 0),
                    ctypes.byref(written),
                )
            )
            checked(set_cursor(console, Coordinate(0, 0)))

        def capture():
            width, height = min(initial.size.x, 80), min(initial.size.y, 5)
            cells = (Cell * (width * height))()
            rectangle = Rectangle(0, 0, width - 1, height - 1)
            checked(
                read(
                    console,
                    cells,
                    Coordinate(width, height),
                    Coordinate(0, 0),
                    ctypes.byref(rectangle),
                )
            )
            return [(cell.character.unicode, cell.attributes) for cell in cells]

        root = Path(root)
        (root / "pyproject.toml").write_text(
            '[project]\nname="demo"\nversion="1.0"\n[build-system]\nrequires=["uv_build>=0.13,<0.14"]\nbuild-backend="uv_build"\n'
        )
        output = root / "out"
        output.mkdir()
        os.chdir(root)
        os.environ.update(
            RUST_LOG="debug",
            UV_PREVIEW="1",
            TERM="xterm-256color",
            CLICOLOR="1",
            CLICOLOR_FORCE="1",
        )
        for name in ["NO_COLOR", "CI"]:
            os.environ.pop(name, None)
        clear()
        filename = subprocess.check_output(
            [reference, "prepare-metadata-for-build-wheel", str(output)], text=True
        ).strip()
        expected = capture()
        shutil.rmtree(output / filename)
        clear()
        actual_filename = backend.prepare_metadata_for_build_wheel(str(output))
        actual = capture()
        assert actual_filename == filename
        assert actual == expected, (actual, expected)
        assert any(attributes != initial.attributes for _, attributes in expected), (
            "The native reference did not exercise console colors: "
            + repr("".join(character for character, _ in expected))
        )
        (root / "console-result.json").write_text(
            json.dumps(
                {
                    "text_and_attributes_equal": True,
                    "colored_cells": sum(
                        attributes != initial.attributes for _, attributes in actual
                    ),
                }
            )
            + "\n"
        )
    finally:
        if console not in {None, ctypes.c_void_p(-1).value}:
            close_handle(console)
        release()


@unittest.skipUnless(os.name == "nt", "This probe uses Win32 consoles")
class WindowsTerminalParity(unittest.TestCase):
    def test_debug_console_text_and_colors(self):
        scratch = Path(
            os.environ.get("TMPDIR", Path.home() / "code/tmp/uv-build-slim-poc/tmp")
        )
        with tempfile.TemporaryDirectory(dir=str(scratch)) as temporary:
            result = subprocess.run(
                [
                    sys.executable,
                    str(Path(__file__).resolve()),
                    "--probe",
                    temporary,
                    os.environ["UV_SLIM_REFERENCE"],
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            evidence = json.loads((Path(temporary) / "console-result.json").read_text())
            self.assertTrue(evidence["text_and_attributes_equal"])


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--probe":
        probe(sys.argv[2], sys.argv[3])
    else:
        unittest.main()
