---
title: Using uv with PyTorch
description:
  Use uv with PyTorch to install packages, configure platforms and accelerators, and add GPU-enabled
  extensions.
---

# Using uv with PyTorch

[PyTorch](https://pytorch.org/) supports deep learning research and development. Use uv to manage
PyTorch projects and dependencies across Python versions and environments. You can also select an
accelerator, such as CPU-only or CUDA.

!!! note

    Some features in this guide require uv version 0.5.3 or later. Update uv before you configure PyTorch.

## Installing PyTorch

PyTorch packages have several uncommon characteristics:

- PyTorch hosts many wheels on dedicated indexes instead of the Python Package Index (PyPI). To
  install these wheels, configure your project to use the appropriate PyTorch index.
- PyTorch produces separate builds for each accelerator, such as CPU-only or CUDA. Python packages
  do not have a standard way to specify accelerators. Therefore, PyTorch identifies each accelerator
  in the local version specifier, such as `2.11.0+cpu` or `2.11.0+cu130`.
- PyTorch publishes builds for different accelerators to different indexes. For example, it
  publishes `+cpu` builds on https://download.pytorch.org/whl/cpu and `+cu130` builds on
  https://download.pytorch.org/whl/cu130.

Your packaging configuration depends on the platforms and accelerators that your project supports.

Run `uv init --python 3.14` and then `uv add torch torchvision` to create this default
configuration.

This configuration installs PyTorch from PyPI. PyPI provides CPU-only wheels for Windows and macOS,
and GPU-accelerated wheels for Linux. In PyTorch 2.11.0, the Linux wheels target CUDA 13.0:

```toml
[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.14"
dependencies = [
  "torch>=2.11.0",
  "torchvision>=0.26.0",
]
```

This configuration uses CPU builds on Windows and macOS, and CUDA-enabled builds on Linux. To
support other platforms or accelerators, change the project configuration.

## Using a PyTorch index

You might need the same PyTorch variant on every platform. For example, you might need CPU-only
builds on Linux as well as Windows and macOS.

First, add the appropriate PyTorch index to `pyproject.toml`:

=== "CPU-only"

    ```toml
    [[tool.uv.index]]
    name = "pytorch-cpu"
    url = "https://download.pytorch.org/whl/cpu"
    explicit = true
    ```

=== "CUDA 11.8"

    ```toml
    [[tool.uv.index]]
    name = "pytorch-cu118"
    url = "https://download.pytorch.org/whl/cu118"
    explicit = true
    ```

=== "CUDA 12.6"

    ```toml
    [[tool.uv.index]]
    name = "pytorch-cu126"
    url = "https://download.pytorch.org/whl/cu126"
    explicit = true
    ```

=== "CUDA 12.8"

    ```toml
    [[tool.uv.index]]
    name = "pytorch-cu128"
    url = "https://download.pytorch.org/whl/cu128"
    explicit = true
    ```

=== "CUDA 13.0"

    ```toml
    [[tool.uv.index]]
    name = "pytorch-cu130"
    url = "https://download.pytorch.org/whl/cu130"
    explicit = true
    ```

=== "ROCm 7.2"

    ```toml
    [[tool.uv.index]]
    name = "pytorch-rocm"
    url = "https://download.pytorch.org/whl/rocm7.2"
    explicit = true
    ```

=== "Intel GPUs"

    ```toml
    [[tool.uv.index]]
    name = "pytorch-xpu"
    url = "https://download.pytorch.org/whl/xpu"
    explicit = true
    ```

Set `explicit = true` so that uv uses the PyTorch index _only_ for packages that explicitly select
it. Use this index for `torch`, `torchvision`, and other PyTorch packages. General dependencies,
such as `jinja2`, should continue to come from the default index, PyPI.

<!-- TODO(tk): Show `uv add --index <name>` once index-by-name is stable. -->

Next, configure `torch` and `torchvision` to use the selected index:

=== "CPU-only"

    ```toml
    [tool.uv.sources]
    torch = [
      { index = "pytorch-cpu" },
    ]
    torchvision = [
      { index = "pytorch-cpu" },
    ]
    ```

=== "CUDA 11.8"

    PyTorch does not publish CUDA builds for macOS. Use `sys_platform` to select the PyTorch index
    on Linux and Windows. On macOS, uv falls back to PyPI:

    ```toml
    [tool.uv.sources]
    torch = [
      { index = "pytorch-cu118", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    torchvision = [
      { index = "pytorch-cu118", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    ```

=== "CUDA 12.6"

    PyTorch does not publish CUDA builds for macOS. Use `sys_platform` to select the PyTorch index
    on Linux and Windows. On macOS, uv falls back to PyPI:

    ```toml
    [tool.uv.sources]
    torch = [
      { index = "pytorch-cu126", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    torchvision = [
      { index = "pytorch-cu126", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    ```

=== "CUDA 12.8"

    PyTorch does not publish CUDA builds for macOS. Use `sys_platform` to select the PyTorch index
    on Linux and Windows. On macOS, uv falls back to PyPI:

    ```toml
    [tool.uv.sources]
    torch = [
      { index = "pytorch-cu128", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    torchvision = [
      { index = "pytorch-cu128", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    ```

=== "CUDA 13.0"

    PyTorch does not publish CUDA builds for macOS. Use `sys_platform` to select the PyTorch index
    on Linux and Windows. On macOS, uv falls back to PyPI:

    ```toml
    [tool.uv.sources]
    torch = [
      { index = "pytorch-cu130", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    torchvision = [
      { index = "pytorch-cu130", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    ```

=== "ROCm 7.2"

    PyTorch does not publish ROCm builds for macOS or Windows. Use `sys_platform` to select the
    PyTorch index only on Linux. On macOS and Windows, uv falls back to PyPI:

    ```toml
    [tool.uv.sources]
    torch = [
      { index = "pytorch-rocm", marker = "sys_platform == 'linux'" },
    ]
    torchvision = [
      { index = "pytorch-rocm", marker = "sys_platform == 'linux'" },
    ]
    # ROCm support relies on both Triton packages, which should also be installed from
    # the PyTorch index (and included in `project.dependencies`).
    pytorch-triton-rocm = [
      { index = "pytorch-rocm", marker = "sys_platform == 'linux'" },
    ]
    triton-rocm = [
      { index = "pytorch-rocm", marker = "sys_platform == 'linux'" },
    ]
    ```

=== "Intel GPUs"

    PyTorch does not publish Intel GPU builds for macOS. Use `sys_platform` to select the PyTorch
    index on Linux and Windows. On macOS, uv falls back to PyPI:

    ```toml
    [tool.uv.sources]
    torch = [
      { index = "pytorch-xpu", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    torchvision = [
      { index = "pytorch-xpu", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    # Intel GPU support relies on `triton-xpu`, which should also be installed from the PyTorch index
    # (and included in `project.dependencies`).
    triton-xpu = [
      { index = "pytorch-xpu", marker = "sys_platform == 'linux' or sys_platform == 'win32'" },
    ]
    ```

This complete project configuration uses CPU-only PyTorch builds on all platforms:

```toml
[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.14.0"
dependencies = [
  "torch>=2.11.0",
  "torchvision>=0.26.0",
]

[tool.uv.sources]
torch = [
    { index = "pytorch-cpu" },
]
torchvision = [
    { index = "pytorch-cpu" },
]

[[tool.uv.index]]
name = "pytorch-cpu"
url = "https://download.pytorch.org/whl/cpu"
explicit = true
```

## Configuring accelerators with environment markers

You might need CPU-only builds on macOS and Windows, but CUDA-enabled builds on Linux.

Use environment markers in `tool.uv.sources` to select an index for each platform. This
configuration uses CUDA-enabled builds on Linux and CPU-only builds on all other platforms:

```toml
[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.14.0"
dependencies = [
  "torch>=2.11.0",
  "torchvision>=0.26.0",
]

[tool.uv.sources]
torch = [
  { index = "pytorch-cpu", marker = "sys_platform != 'linux'" },
  { index = "pytorch-cu130", marker = "sys_platform == 'linux'" },
]
torchvision = [
  { index = "pytorch-cpu", marker = "sys_platform != 'linux'" },
  { index = "pytorch-cu130", marker = "sys_platform == 'linux'" },
]

[[tool.uv.index]]
name = "pytorch-cpu"
url = "https://download.pytorch.org/whl/cpu"
explicit = true

[[tool.uv.index]]
name = "pytorch-cu130"
url = "https://download.pytorch.org/whl/cu130"
explicit = true
```

This configuration uses AMD GPU builds on Linux. On Windows and macOS, it falls back to CPU-only
builds from PyPI:

```toml
[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.14.0"
dependencies = [
  "torch>=2.11.0",
  "torchvision>=0.26.0",
  "pytorch-triton-rocm>=3.5.1 ; sys_platform == 'linux'",
  "triton-rocm>=3.6.0 ; sys_platform == 'linux'",
]

[tool.uv.sources]
torch = [
  { index = "pytorch-rocm", marker = "sys_platform == 'linux'" },
]
torchvision = [
  { index = "pytorch-rocm", marker = "sys_platform == 'linux'" },
]
pytorch-triton-rocm = [
  { index = "pytorch-rocm", marker = "sys_platform == 'linux'" },
]
triton-rocm = [
  { index = "pytorch-rocm", marker = "sys_platform == 'linux'" },
]

[[tool.uv.index]]
name = "pytorch-rocm"
url = "https://download.pytorch.org/whl/rocm7.2"
explicit = true
```

For Intel GPU builds, use this configuration:

```toml
[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.14.0"
dependencies = [
  "torch>=2.11.0",
  "torchvision>=0.26.0",
  "triton-xpu>=3.7.0 ; sys_platform == 'win32' or sys_platform == 'linux'",
]

[tool.uv.sources]
torch = [
  { index = "pytorch-xpu", marker = "sys_platform == 'win32' or sys_platform == 'linux'" },
]
torchvision = [
  { index = "pytorch-xpu", marker = "sys_platform == 'win32' or sys_platform == 'linux'" },
]
triton-xpu = [
  { index = "pytorch-xpu", marker = "sys_platform == 'win32' or sys_platform == 'linux'" },
]

[[tool.uv.index]]
name = "pytorch-xpu"
url = "https://download.pytorch.org/whl/xpu"
explicit = true
```

## Configuring accelerators with optional dependencies

You can let users select a CPU-only or CUDA-enabled build with an extra. For example, users can run
`uv sync --extra cpu` or `uv sync --extra cu130`.

Use extra markers in `tool.uv.sources` to select an index for each extra. This configuration uses
CPU-only builds for `uv sync --extra cpu` and CUDA-enabled builds for `uv sync --extra cu130`:

```toml
[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.14.0"
dependencies = []

[project.optional-dependencies]
cpu = [
  "torch>=2.11.0",
  "torchvision>=0.26.0",
]
cu130 = [
  "torch>=2.11.0",
  "torchvision>=0.26.0",
]

[tool.uv]
conflicts = [
  [
    { extra = "cpu" },
    { extra = "cu130" },
  ],
]

[tool.uv.sources]
torch = [
  { index = "pytorch-cpu", extra = "cpu" },
  { index = "pytorch-cu130", extra = "cu130" },
]
torchvision = [
  { index = "pytorch-cpu", extra = "cpu" },
  { index = "pytorch-cu130", extra = "cu130" },
]

[[tool.uv.index]]
name = "pytorch-cpu"
url = "https://download.pytorch.org/whl/cpu"
explicit = true

[[tool.uv.index]]
name = "pytorch-cu130"
url = "https://download.pytorch.org/whl/cu130"
explicit = true
```

!!! note

    PyTorch does not provide GPU-accelerated builds for macOS. Therefore, installation fails on
    macOS if you enable the `cu130` extra.

## Installing GPU-enabled PyTorch extensions

Many PyTorch packages include GPU-enabled extensions for specific CUDA and PyTorch versions. To
build these packages from source, you often need the CUDA development toolkit and extra build
configuration.

The [Astral GPU indexes](https://wheels.astral.sh/) provide pre-built wheels for several Python,
CUDA, and PyTorch versions. Supported packages include `flash-attn`, `deepspeed`, `deep-gemm`,
`torch-scatter`, and `vllm`.

To install `flash-attn` from the index for CUDA 12.8, run:

```console
$ uv add flash-attn --index astral-cu128=https://wheels.astral.sh/simple/cu128/
```

This command adds `flash-attn` to the project dependencies and configures the Astral GPU index. It
also pins `flash-attn` to that index.

As with the PyTorch indexes, set `explicit = true`. This restricts the Astral GPU index to packages
that explicitly select it:

```toml title="pyproject.toml"
[tool.uv.sources]
flash-attn = { index = "astral-cu128" }

[[tool.uv.index]]
name = "astral-cu128"
url = "https://wheels.astral.sh/simple/cu128/"
explicit = true
```

Each Astral GPU index targets a specific CUDA version. Its wheels also target specific PyTorch
versions. For example, a wheel with the local version `+cu.12.8.torch.2.11` targets CUDA 12.8 and
PyTorch 2.11. Select an index and wheel that match your Python version, platform, CUDA version, and
PyTorch installation.

To find available packages and supported CUDA and PyTorch versions, see the
[Astral GPU indexes](https://wheels.astral.sh/).

## The `uv pip` interface

The previous examples use the uv project interface, such as `uv lock`, `uv sync`, and `uv run`. You
can also install PyTorch with the `uv pip` interface.

Use the [PyTorch installation interface](https://pytorch.org/get-started/locally/) to find the pip
command for your target configuration. For example, install stable, CPU-only PyTorch on Linux with:

```shell
$ pip3 install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cpu
```

To use the same workflow with uv, replace `pip3` with `uv pip`:

```shell
$ uv pip install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cpu
```

## Automatic backend selection

To let uv select the PyTorch index, use `--torch-backend=auto` or set `UV_TORCH_BACKEND=auto`:

```shell
$ # With a command-line argument.
$ uv pip install torch --torch-backend=auto

$ # With an environment variable.
$ UV_TORCH_BACKEND=auto uv pip install torch
```

uv checks for an installed CUDA driver, AMD GPU versions, and Intel GPUs. It selects the most
compatible PyTorch index for packages such as `torch` and `torchvision`. If uv does not find a
compatible GPU, it uses the CPU-only index. Existing index configuration still applies to packages
outside the PyTorch ecosystem.

To select a specific backend, such as CUDA 13.0, use `--torch-backend=cu130` or set
`UV_TORCH_BACKEND=cu130`:

```shell
$ # With a command-line argument.
$ uv pip install torch torchvision --torch-backend=cu130

$ # With an environment variable.
$ UV_TORCH_BACKEND=cu130 uv pip install torch torchvision
```

`--torch-backend` is available only in the `uv pip` interface.
