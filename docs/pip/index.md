# The pip interface

uv replaces common `pip`, `pip-tools`, and `virtualenv` commands. These commands work directly with
virtual environments. In contrast, uv's primary interfaces manage virtual environments
automatically. The `uv pip` interface provides uv's speed and functionality without requiring a
change from existing `pip` or `pip-tools` workflows.

Use the following sections to get started with `uv pip`:

- [Creating and using environments](./environments.md)
- [Installing and managing packages](./packages.md)
- [Inspecting environments and packages](./inspection.md)
- [Declaring package dependencies](./dependencies.md)
- [Locking and syncing environments](./compile.md)

These commands do not _exactly_ match the interfaces and behavior of the original tools. Differences
are more likely in less common workflows. See the [pip-compatibility guide](./compatibility.md) for
details.

!!! important

    uv does not depend on or run pip. The pip interface provides low-level commands that match pip's
    interface. Its name distinguishes these commands from uv's higher-level commands.
