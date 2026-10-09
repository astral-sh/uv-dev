"""
Based on
https://github.com/pypa/pip/blob/3820b0e52c7fed2b2c43ba731b718f316e6816d1/src/pip/_internal/operations/install/wheel.py#L612-L623

pip silently just swallows all pyc compilation errors, but `python -m compileall` does
not have such a flag, so we adapt the pip code. This is relevant, e.g., for
`debugpy-1.5.1-cp38-cp38-manylinux_2_5_x86_64.manylinux1_x86_64.manylinux_2_12_x86_64.manylinux2010_x86_64`,
which contains some vendored Python 2 code which fails to compile.
"""

import compileall
import os
import py_compile
import sys
import warnings

with warnings.catch_warnings():
    warnings.filterwarnings("ignore")

    # Successful launch check
    print("Ready")

    # https://docs.python.org/3/library/py_compile.html#py_compile.PycInvalidationMode
    # TIMESTAMP, CHECKED_HASH, UNCHECKED_HASH
    invalidation_mode = os.environ.get("PYC_INVALIDATION_MODE")
    if invalidation_mode is not None:
        try:
            invalidation_mode = py_compile.PycInvalidationMode[invalidation_mode]
        except KeyError:
            invalidation_modes = ", ".join(
                '"' + x.name + '"' for x in py_compile.PycInvalidationMode
            )
            print(
                f'Invalid value for PYC_INVALIDATION_MODE "{invalidation_mode}", '
                f"valid are {invalidation_modes}: ",
                file=sys.stderr,
            )
            sys.exit(1)
    if invalidation_mode is None:
        try:
            invalidation_mode = py_compile._get_default_invalidation_mode()
        except AttributeError:
            invalidation_mode = None  # guard against implementation details

    # Unlike pip, we will usually set force=False. It's unclear why pip sets force=True, but it
    # doesn't matter much for them, as pip only compiles newly installed files.
    force = False
    if invalidation_mode != py_compile.PycInvalidationMode.TIMESTAMP:
        # Note that compileall has undesirable, arguably buggy behaviour. Even if invalidation_mode
        # is hash based, compileall will not recompile the file if the existing pyc is timestamp
        # based and has a matching mtime (unless force=True).
        force = True

    source_root, destination_root = sys.argv[1:] if len(sys.argv) > 1 else (None, None)
    if destination_root is not None:
        # Recompile existing bytecode too, so code objects cannot retain temporary source paths.
        force = True

    # In rust, we provide one line per file to compile.
    for path in sys.stdin:
        # Remove trailing newlines.
        path = path.strip()
        if not path:
            continue
        # Unlike pip, we set quiet=2, so we don't have to capture stdout.
        # We'd like to show those errors, but given that pip thinks that's totally fine,
        # we can't really change that.
        display_directory = None
        if destination_root is not None:
            relative = os.path.relpath(os.path.dirname(path), source_root)
            if relative == os.pardir or relative.startswith(os.pardir + os.sep):
                raise ValueError("bytecode source is outside the staged directory")
            display_directory = os.path.normpath(
                os.path.join(destination_root, relative)
            )
        success = compileall.compile_file(
            path,
            ddir=display_directory,
            invalidation_mode=invalidation_mode,
            force=force,
            quiet=2,
        )
        # We're ready for the next file.
        print(path)
