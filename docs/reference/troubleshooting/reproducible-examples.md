# Reproducible examples

## Why reproducible examples are important

A minimal reproducible example (MRE) helps maintainers fix bugs. Without an example that reproduces
the problem, a maintainer cannot investigate it or verify a fix. Extra content that does not relate
to the issue makes the cause harder to identify.

## How to write a reproducible example

A reproducible example provides all the information needed to reproduce an issue. This includes:

- The platform, including the operating system and architecture
- Any relevant system state (e.g., explicitly set environment variables)
- The version of uv
- The version of other relevant tools
- The relevant files, such as `uv.lock` and `pyproject.toml`
- The commands to run

A minimal reproduction includes only the dependencies, settings, and files needed to show the
problem. Testing the reproduction before sharing it confirms that it works. Verbose logs can show
important differences between systems. A [Gist](https://gist.github.com) can store very long logs.

The following [strategies](#strategies-for-reproducible-examples) describe how to create and share
reproducible examples.

!!! tip

    [Stack Overflow](https://stackoverflow.com/help/minimal-reproducible-example) provides a guide
    to creating minimal reproducible examples.

## Strategies for reproducible examples

### Docker image

A Docker image is often the best way to share a reproducible example because it is self-contained.
The state of the host system does not affect the problem.

!!! note

    A Docker image usually requires an issue that can be reproduced on Linux. Some macOS issues also
    occur on Linux, but others depend on the operating system. Windows containers are possible but
    uncommon. A [script](#script) is better for issues that require macOS or Windows.

A Docker MRE can start from one of
[uv's Docker images](../../guides/integration/docker.md#available-images). The image should specify
an exact uv version:

```Dockerfile
FROM ghcr.io/astral-sh/uv:0.12.0-debian-slim
```

Docker images are isolated, but builds use the host architecture by default. An explicit platform
ensures that the reproduction uses the expected architecture. uv publishes images for `linux/amd64`,
such as Intel or AMD, and `linux/arm64`, such as Apple M-series or ARM:

```Dockerfile
FROM --platform=linux/amd64 ghcr.io/astral-sh/uv:0.12.0-debian-slim
```

Docker images work well for issues that can be reproduced with commands:

```Dockerfile
FROM --platform=linux/amd64 ghcr.io/astral-sh/uv:0.12.0-debian-slim

RUN uv init /mre
WORKDIR /mre
RUN uv add pydantic
RUN uv sync
RUN uv run -v python -c "import pydantic"
```

An image can also include files directly:

```Dockerfile
FROM --platform=linux/amd64 ghcr.io/astral-sh/uv:0.12.0-debian-slim

COPY <<EOF /mre/pyproject.toml
[project]
name = "example"
version = "0.1.0"
description = "Add your description here"
readme = "README.md"
requires-python = ">=3.12"
dependencies = ["pydantic"]
EOF

WORKDIR /mre
RUN uv lock
```

A [Git repository](#git-repository) is a better choice when the reproduction requires many files.
The repository can also include a `Dockerfile`.

Docker build logs help explain the failure. Disabling caching and using plain progress output shows
more information from each build step:

```console
docker build . --progress plain --no-cache
```

### Script

For platform-specific bugs that cannot be reproduced in a [container](#docker-image), a script can
show the commands needed to reproduce the issue:

```bash
uv init
uv add pydantic
uv sync
uv run -v python -c "import pydantic"
```

A [Git repository](#git-repository) can share reproductions that require many files.

The script should include the complete error message and _verbose_ logs from the `-v` option.

Reports should also describe any external state that the script requires. For example, a Windows
report may need to specify a Python version installed with `choco` and PowerShell 6.2.

### Git repository

A Git repository reproduction should include a [script](#script) or [Dockerfile](#docker-image) that
reproduces the issue. The script should first clone the repository and check out a specific commit:

```console
$ git clone https://github.com/<user>/<project>.git
$ cd <project>
$ git checkout <commit>
$ <commands to produce error>
```

The [GitHub UI](https://github.com/new) and the `gh` CLI can create a new repository:

```console
$ gh repo create uv-mre-1234 --clone
```

A reproduction repository should _minimize_ its contents and exclude files or settings that do not
relate to the problem.
