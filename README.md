# Remove misleading note from uv documentation

Issue: astral-sh/uv#23065

Classification: bug (documentation)

## Summary

The reporter requests removal of the distroless-image note under “Using the uv image” in the GitLab integration guide. Its `entrypoint: [""]` example implies that clearing the image entrypoint is sufficient to use the image for GitLab CI jobs. The report cites GitLab's shell requirement and astral-sh/uv#9361; it provides no job configuration, platform, GitLab Runner version, uv version, or Python version, and does not report an upgrade regression.

A targeted execution against the published `ghcr.io/astral-sh/uv:0.13.6` image reproduced the missing-shell failure. The image's uv executable works, but bypassing its entrypoint and launching `sh` fails because the image contains no shell. This supports correcting the guide. The image itself behaves consistently with its documented distroless design.

## Classification

This is a documentation correctness bug, despite the issue's question framing. Clearing the entrypoint addresses uv interpreting `sh` as a subcommand; it does not supply the shell required by GitLab's Docker executor. The requested correction concerns the GitLab guide, not adding a shell to distroless images or changing uv's runtime behavior.

The reproduction isolates the shell-launch requirement using the published image filesystem. A complete GitLab CI job was not run, so the observed error is from `chroot`, not a newly captured GitLab Runner or Docker error.

## Related

- astral-sh/uv#9093 — Docs(GitLab): add entrypoint for distroless image (merged pull request). Added the reported note on December 4, 2024. Its discussion explicitly states that GitLab cannot use the distroless image because it lacks `sh`; the author acknowledges changing images between tests. This introduced the note rather than correcting the missing-shell limitation.
- astral-sh/uv#9361 — UV on gitlab CICD has some issue (closed issue). Reports `unrecognized subcommand 'sh'`, followed by `exec: "sh": executable file not found in $PATH` after clearing the entrypoint. A maintainer recommended a non-distroless image, and the reporter confirmed success on November 22, 2024. It corroborates the limitation but closed before the note was added and does not track this documentation correction.
- astral-sh/uv#7054 — ci(docker)!: adjust entrypoint and cmd for inherited images (merged pull request). Changed derived-image entrypoints on September 4, 2024. It did not supply a shell in distroless images or fix the GitLab note; the report is not a regression of that change.
- astral-sh/uv-docker-example#31 — Earlier support report from the reporter of astral-sh/uv#9361. A maintainer explains that the scratch image lacks a shell and recommends a derived image; the discussion then moves to astral-sh/uv#9361.

## Reproduction

**Outcome: reproducible**, for the missing-shell behavior underlying the documentation report.

Environment:

- Ubuntu 24.04.5 LTS, Linux `7.0.0-1017-azure`, x86_64.
- Installed executable on PATH: `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Published image: `ghcr.io/astral-sh/uv:0.13.6`, Linux/amd64; image executable reports `uv 0.13.6 (x86_64-unknown-linux-musl)`.
- Platform manifest digest: `sha256:8b7774a14bb3abde914fda03efcb83f1f7e9163124e5c13e7fb4cc825293f5fa`.
- `skopeo 1.13.3` downloaded the image anonymously into a temporary directory. A rootless user namespace and `chroot` executed commands in its unpacked filesystem, without using the shared Docker daemon.
- Host Python 3.12.3 unpacked the image. Python inside the image is not needed to trigger this failure; shell startup fails first.

The minimal GitLab configuration represented by the reported note is:

```yaml
uv:
  image:
    name: ghcr.io/astral-sh/uv:0.13.6
    entrypoint: [""]
  script:
    - /uv --version
```

This YAML was not submitted to GitLab. The following local procedure reproduces the relevant image commands. All downloaded files and caches are temporary:

```sh
repro_directory=$(mktemp -d /tmp/uv-23065-repro.XXXXXX)
cd "$repro_directory"
mkdir tmp cache config runtime
export TMPDIR="$repro_directory/tmp"
export XDG_CACHE_HOME="$repro_directory/cache"
export XDG_CONFIG_HOME="$repro_directory/config"
export XDG_RUNTIME_DIR="$repro_directory/runtime"
export UV_CACHE_DIR="$repro_directory/cache/uv"
printf '%s\n' '{}' > auth.json
uv --version
skopeo copy --src-no-creds --authfile "$repro_directory/auth.json" \
  --override-os linux --override-arch amd64 \
  docker://ghcr.io/astral-sh/uv:0.13.6 "dir:$repro_directory/image"
python3 - <<'PYTHON'
import json
from pathlib import Path
import tarfile

image = Path('image')
manifest = json.loads((image / 'manifest.json').read_text())
rootfs = Path('rootfs')
rootfs.mkdir()
for layer in manifest['layers']:
    archive_path = image / layer['digest'].split(':')[1]
    with tarfile.open(archive_path) as archive:
        archive.extractall(rootfs, filter='data')
PYTHON
unshare --user --map-root-user chroot "$repro_directory/rootfs" /uv --version
unshare --user --map-root-user chroot "$repro_directory/rootfs" /uv sh -c '/uv --version'
unshare --user --map-root-user chroot "$repro_directory/rootfs" sh -c '/uv --version'
```

Observed results from the three explicit image commands:

1. `/uv --version`: exit 0, `uv 0.13.6 (x86_64-unknown-linux-musl)`. This establishes that the actual image executable runs in the reproduction environment.
2. `/uv sh -c '/uv --version'`: exit 2, `error: unrecognized subcommand 'sh'`. This represents the image's default `/uv` entrypoint receiving the executor's shell command.
3. `sh -c '/uv --version'`, with `/uv` bypassed: exit 127, `/usr/sbin/chroot: failed to run command 'sh': No such file or directory`. Clearing the entrypoint still cannot start the shell.

The downloaded image config specifies `Entrypoint: ["/uv"]` and working directory `/io`. Its layers contain only `uv`, `uvx`, and the empty `io` directory. The execution results, rather than source inspection alone, establish the reported missing-shell behavior. Temporary image files and command logs are retained at `/tmp/uv-23065-repro.ae98WS`.

Existing test coverage: searches of `crates/uv/tests/` and `crates/uv-client/tests/it/` for GitLab image usage, distroless images, Docker execution, shell entrypoints, and missing-executable errors found no direct test of this guide example or its shell requirement. GitLab trusted-publishing tests and Python console-entrypoint tests exercise different behavior. No repository tests were added or run; the validation above used the published container image, with no build.

## Supporting evidence

- At checkout commit `9b11efe914e01b9dfaa53beebc2180f37dafb19b`, `docs/guides/integration/gitlab.md:29` contains the note. The main example at line 24 uses a Python-based derived image.
- `Dockerfile:70` starts the final image with `FROM scratch`, copies only `/uv` and `/uvx`, and sets `ENTRYPOINT ["/uv"]`. The downloaded image contents and execution match this source.
- `docs/guides/integration/docker.md:17` explains that distroless images contain only uv binaries, while derived images include an operating system.
- GitLab Runner's Docker executor documentation says images must provide `sh` or `bash` and `grep`. It describes `entrypoint: [""]` as equivalent to `docker run --entrypoint "" <image> sh -c ...`. The rendered documentation URL returned HTTP 403, but the Markdown source was successfully read at https://raw.githubusercontent.com/gitlabhq/gitlab-runner/main/docs/executors/docker.md, under “Configure a Docker ENTRYPOINT”.

## Related-work search

Related-work searches covered open and closed issues and pull requests, GitLab/distroless/entrypoint/shell terminology, the guide anchor, exact error fragments, linked discussions, patches, and the guide's history. No existing correction was identified. Pull-request keyword search results were incomplete even for known relevant merged changes, so direct linked-pull-request inspection, guide history, and open-pull-request listings supplemented them.

Excluded candidates include astral-sh/uv#8635 (libc/Python discovery after uv starts), astral-sh/uv#16432 (publishing/release documentation), and astral-sh/uv#22542 (unrelated system-installation wording). The inspected comments in the general documentation tracker, astral-sh/uv#5605, did not track this note. These historical discussions support the context; the reproduction conclusion rests on the image execution above.

## Recommended next step

Remove the distroless entrypoint example, or replace it with an explicit explanation that GitLab Docker-executor job images need a shell and should use a suitable derived image. Keep the Python-based example as the supported direction. No uv runtime change is indicated by this reproduction.

Only this handoff and temporary reproduction files were written. No checkout files or existing user state were changed, and no GitHub changes were made.
