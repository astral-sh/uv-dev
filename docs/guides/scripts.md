---
title: Running scripts
description:
  Use uv to run Python scripts, declare dependencies with inline metadata, and improve
  reproducibility.
---

# Running scripts

A Python script is a file that you can run directly, such as with `python <script>.py`. When you run
a script with uv, uv manages its dependencies and environment.

!!! note

    Every Python installation has an environment where you can install packages. Create
    [_virtual_ environments](https://docs.python.org/3/library/venv.html) to isolate the packages
    for each script. uv manages these environments automatically. It prefers
    [declared script dependencies](#declaring-script-dependencies).

## Running a script without dependencies

If your script has no dependencies, run it with `uv run`:

```python title="example.py"
print("Hello world")
```

```console
$ uv run example.py
Hello world
```

<!-- TODO(zanieb): Once we have a `python` shim, note you can execute it with `python` here -->

If your script only uses modules from the standard library, no additional setup is necessary:

```python title="example.py"
import os

print(os.path.expanduser("~"))
```

```console
$ uv run example.py
/Users/astral
```

Add arguments after the script name:

```python title="example.py"
import sys

print(" ".join(sys.argv[1:]))
```

```console
$ uv run example.py test
test

$ uv run example.py hello world!
hello world!
```

You can also read a script from standard input:

```console
$ echo 'print("hello world!")' | uv run -
```

If your shell supports [here-documents](https://en.wikipedia.org/wiki/Here_document), you can run:

```bash
uv run - <<EOF
print("hello world!")
EOF
```

If you run `uv run` in a _project_, uv installs the project before it runs the script. A project is
a directory with a `pyproject.toml` file. If the script does not depend on the project, use
`--no-project` to skip the installation:

```console
$ # Note: the `--no-project` flag must be provided _before_ the script name.
$ uv run --no-project example.py
```

For details, see the [projects guide](./projects.md).

## Running a script with dependencies

If your script requires other packages, install those packages in the script environment. uv creates
this environment when necessary instead of using a manually managed virtual environment. You must
declare the packages that your script requires. Use a [project](./projects.md) or
[inline metadata](#declaring-script-dependencies) to declare dependencies. You can also request
dependencies each time you run the script.

For example, this script requires `rich`:

```python title="example.py"
import time
from rich.progress import track

for i in track(range(20), description="For example:"):
    time.sleep(0.05)
```

If you do not specify the dependency, the script fails:

```console
$ uv run --no-project example.py
Traceback (most recent call last):
  File "/Users/astral/example.py", line 2, in <module>
    from rich.progress import track
ModuleNotFoundError: No module named 'rich'
```

To request the dependency, use `--with`:

```console
$ uv run --with rich example.py
For example: ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 100% 0:00:01
```

To request specific versions, add version constraints:

```console
$ uv run --with 'rich>12,<13' example.py
```

To request multiple dependencies, repeat the `--with` option.

If you run `uv run` in a _project_, uv includes these dependencies and the project dependencies. To
exclude project dependencies, use `--no-project`.

## Creating a Python script

Python defines a standard format for
[inline script metadata](https://packaging.python.org/en/latest/specifications/inline-script-metadata/#inline-script-metadata).
Use this metadata to select a Python version and define dependencies. To create a script with inline
metadata, run `uv init --script`:

```console
$ uv init --script example.py --python 3.12
```

## Declaring script dependencies

Inline metadata declares script dependencies directly in the script.

Use `uv add --script` to add or update the script dependencies:

```console
$ uv add --script example.py 'requests<3' 'rich'
```

This command adds a `script` section at the top of the file. The section declares dependencies in
TOML format:

```python title="example.py"
# /// script
# dependencies = [
#   "requests<3",
#   "rich",
# ]
# ///

import requests
from rich.pretty import pprint

resp = requests.get("https://peps.python.org/api/peps.json")
data = resp.json()
pprint([(k, v["title"]) for k, v in data.items()][:10])
```

uv automatically creates an environment with the dependencies that the script requires:

```console
$ uv run example.py
[
│   ('1', 'PEP Purpose and Guidelines'),
│   ('2', 'Procedure for Adding New Modules'),
│   ('3', 'Guidelines for Handling Bug Reports'),
│   ('4', 'Deprecation of Standard Modules'),
│   ('5', 'Guidelines for Language Evolution'),
│   ('6', 'Bug Fix Releases'),
│   ('7', 'Style Guide for C Code'),
│   ('8', 'Style Guide for Python Code'),
│   ('9', 'Sample Plaintext PEP Template'),
│   ('10', 'Voting Guidelines')
]
```

!!! important

    If a script includes inline metadata, uv ignores project dependencies. This behavior applies
    even when you [run the script in a _project_](../concepts/projects/run.md). You do not need
    `--no-project`.

uv also uses Python version requirements from inline metadata:

```python title="example.py"
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///

# Use some syntax added in Python 3.12
type Point = tuple[float, float]
print(Point)
```

!!! note

    Include the `dependencies` field even if it is empty.

The `uv run` command finds and uses the required Python version. If the version is not installed, uv
downloads it. For details, see [Python versions](../concepts/python-versions.md).

## Using a shebang to create an executable file

Add a shebang to run a script without entering `uv run`. You can then run scripts in your `PATH` or
the current directory.

For example, create a file named `greet` with this content:

```python title="greet"
#!/usr/bin/env -S uv run --script

print("Hello, world!")
```

Make the script executable with `chmod +x greet`. Then run the script:

```console
$ ./greet
Hello, world!
```

You can also declare dependencies in a script with a shebang:

```python title="example"
#!/usr/bin/env -S uv run --script
#
# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx"]
# ///

import httpx

print(httpx.get("https://example.com"))
```

## Using alternative package indexes

To resolve dependencies from an alternative [package index](../concepts/indexes.md), use `--index`:

```console
$ uv add --index "https://example.com/simple" --script example.py 'requests<3' 'rich'
```

This command adds the package index to the inline metadata:

```python
# [[tool.uv.index]]
# url = "https://example.com/simple"
```

If the package index requires authentication, see the [package index](../concepts/indexes.md)
documentation.

## Locking dependencies

uv can lock dependencies for PEP 723 scripts with the `uv.lock` file format. Unlike projects,
scripts require an explicit `uv lock` command:

```console
$ uv lock --script example.py
```

The `uv lock --script` command creates a `.lock` file next to the script, such as `example.py.lock`.

After you lock the script, commands reuse the locked dependencies and update the lockfile if
necessary. These commands include `uv run --script`, `uv add --script`, `uv export --script`, and
`uv tree --script`.

If a script does not have a lockfile, commands such as `uv export --script` still work. These
commands do not create a lockfile.

## Improving reproducibility

To improve reproducibility, add `exclude-newer` to the `tool.uv` section of inline script metadata.
This field limits uv to distributions that were released before a specific date. This limit makes
later script runs more reproducible.

Specify the date as an [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339.html) timestamp, such as
`2006-12-02T02:07:43Z`.

```python title="example.py"
# /// script
# dependencies = [
#   "requests",
# ]
# [tool.uv]
# exclude-newer = "2023-10-16T00:00:00Z"
# ///

import requests

print(requests.__version__)
```

## Using different Python versions

You can request a Python version each time you run a script. For example:

```python title="example.py"
import sys

print(".".join(map(str, sys.version_info[:3])))
```

```console
$ # Use the default Python version, may differ on your machine
$ uv run example.py
3.12.6
```

```console
$ # Use a specific Python version
$ uv run --python 3.10 example.py
3.10.15
```

For details, see the [Python version request](../concepts/python-versions.md#requesting-a-version)
documentation.

## Using GUI scripts

On Windows, uv uses `pythonw` to run scripts with the `.pyw` extension:

```python title="example.pyw"
from tkinter import Tk, ttk

root = Tk()
root.title("uv")
frm = ttk.Frame(root, padding=10)
frm.grid()
ttk.Label(frm, text="Hello World").grid(column=0, row=0)
root.mainloop()
```

```console
PS> uv run example.pyw
```

![Run Result](../assets/uv_gui_script_hello_world.png){: style="height:50px;width:150px"}

GUI scripts can also use dependencies:

```python title="example_pyqt.pyw"
import sys
from PyQt5.QtWidgets import QApplication, QWidget, QLabel, QGridLayout

app = QApplication(sys.argv)
widget = QWidget()
grid = QGridLayout()

text_label = QLabel()
text_label.setText("Hello World!")
grid.addWidget(text_label)

widget.setLayout(grid)
widget.setGeometry(100, 100, 200, 50)
widget.setWindowTitle("uv")
widget.show()
sys.exit(app.exec_())
```

```console
PS> uv run --with PyQt5 example_pyqt.pyw
```

![Run Result](../assets/uv_gui_script_hello_world_pyqt.png){: style="height:50px;width:150px"}

## Next steps

For details about `uv run`, see the [command reference](../reference/cli.md#uv-run).

Next, learn how to [run and install tools](./tools.md).
