# ghcr.io/astral-sh/uv:latest causing Docker build failure

Issue: astral-sh/uv#21576

Classification: question

## Summary

The reporter copies `/uv` and `/uvx` from `ghcr.io/astral-sh/uv:latest` into a
`python:3.12-slim` stage, then sees Docker fail while starting a container because `/bin/sh` is
missing. They ask whether `latest` changed and for the previously working version or digest.

The tag did change shortly before the report. `latest` now resolves to uv 0.12.12, published on
2026-09-09, with multi-platform index digest
`sha256:73d2665b478d8fa2de1cf105c6841f8e9cb6b09e568fc7700440c09f8fcd7ac4`. The immediately
preceding release is `ghcr.io/astral-sh/uv:0.12.11`, whose multi-platform index digest is
`sha256:79c6f4776b851471cc73b7d21d0cc834bb94383c292e83640d27eff512864df7`.
A maintainer recommends pinning released version tags and specifically identifies 0.12.11 as the
version to try before 0.12.12.

The source image is intentionally distroless and has never been expected to contain `/bin/sh`.
That does not by itself explain this report: an external image referenced by `COPY --from` is read
as a filesystem source, not executed as the active build stage. A minimal build using the current
`latest`, `python:3.12-slim`, the reported `COPY`, and a subsequent `RUN test -x /bin/sh && uv
--version` succeeded and printed uv 0.12.12. The full Dockerfile and the BuildKit line identifying
the failing step are therefore needed to determine which stage is trying to execute `/bin/sh`.

A maintainer has now marked the report as lacking enough information to reproduce and requested a
minimal reproducible example. The requested follow-up includes the uv version, operating system,
command, and output; for this Docker failure, the complete Dockerfile and full numbered BuildKit
output remain the essential missing pieces.

The reporter initially found that changing the image reference from `latest`/0.12.12 to
`ghcr.io/astral-sh/uv:0.12.11` resolved the container-initialization error in their build. By the
following day, however, the original setup also worked again and the reporter could no longer
reproduce the failure. The rollback is therefore a historical workaround, not stable evidence that
0.12.12 caused the error. The reporter plans to provide an MRE if the issue recurs. They also report
a separate package-installation timeout that stopped when `ENV UV_COMPILE_BYTECODE=1` was removed;
no error output or reproduction was provided for that secondary behavior.

A maintainer notes that uv's basic images have never included `sh` and suggests that the reporter's
parent image may instead have changed. This is an unconfirmed hypothesis, but it makes the exact
digest resolved for the mutable `python:3.12-slim` tag in the failing build another important input
for comparison.

After the reporter confirmed that the failure had disappeared, a maintainer ended the current
investigation and invited them to reopen astral-sh/uv#21576 if they obtain a reproduction. No
further repository action is pending without an MRE.

## Reproduction

Outcome: `needs_more_information`.

On Linux x86_64 with Docker Engine 28.0.4, Buildx 0.37.0, and the installed uv 0.12.12, the
reported installation pattern was reconstructed in a clean temporary directory:

```dockerfile
FROM python:3.12-slim
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/
RUN test -x /bin/sh && uv --version && python --version
```

```console
$ docker buildx build --pull --no-cache --progress=plain --load .
#7 [stage-0 2/3] COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/
#7 DONE 0.1s
#8 [stage-0 3/3] RUN test -x /bin/sh && uv --version && python --version
#8 0.143 uv 0.12.12 (x86_64-unknown-linux-musl)
#8 0.145 Python 3.12.14
#8 DONE 0.2s
```

BuildKit resolved `python:3.12-slim` to
`sha256:78387bc3881b8273120a12ebe6c1ab22b018ccc2c9adf565ae1ac9b536e184ea` and `latest` to
`sha256:73d2665b478d8fa2de1cf105c6841f8e9cb6b09e568fc7700440c09f8fcd7ac4`. The same build with
`ghcr.io/astral-sh/uv:0.12.11` at
`sha256:79c6f4776b851471cc73b7d21d0cc834bb94383c292e83640d27eff512864df7` also succeeded and
printed uv 0.12.11. Thus the supplied `COPY` line does not reproduce the reported failure, and
pinning the immediately previous image does not change this result.

A focused stage-selection variant does reproduce the exact error:

```dockerfile
ARG UV_IMAGE=ghcr.io/astral-sh/uv:latest
FROM ${UV_IMAGE}
RUN uv --version
```

It fails before uv runs with `exec: "/bin/sh": stat /bin/sh: no such file or directory`. The same
variant fails identically with 0.12.11. This is expected because both tags use the repository's
`FROM scratch` distroless image, but it does not establish that the reporter's omitted Dockerfile
selects the uv image as the active stage.

The 0.12.12 changelog and astral-sh/uv#21558 contain a release/version update, code-signing notes,
and an unrelated hash-cutoff fix; they contain no Docker image layout change. Literal searches of
`crates/uv/tests/` and `crates/uv-client/tests/it/` found no integration test that builds the
published Docker image or covers this external-stage COPY pattern.

The reporter initially reported that the same build succeeded when pinned to 0.12.11, but now
cannot reproduce the failure with the original setup either. No Dockerfile or comparative logs were
captured while it failed. If it recurs, diagnosis requires the complete Dockerfile, full BuildKit
output including the numbered failing instruction, the build command and build context/target, the
resolved parent-image digest, and output from both 0.12.12 and 0.12.11. Those details will identify
which active stage is missing `/bin/sh` and whether the mutable parent image differs; the provided
line alone only reads files from the distroless image.

The repository bot subsequently recorded that a maintainer considers the issue non-reproducible
with the information provided and requested an MRE, including the uv version, operating system,
command, and output. This confirms that investigation is waiting on reporter-supplied reproduction
details rather than on an identified uv fix. Once the reporter confirmed they could no longer
reproduce the failure, a maintainer invited them to reopen astral-sh/uv#21576 if it recurs.

## Draft response

`latest` was updated on September 9 and currently resolves to uv 0.12.12 at
`sha256:73d2665b478d8fa2de1cf105c6841f8e9cb6b09e568fc7700440c09f8fcd7ac4`. The immediately
previous image is `ghcr.io/astral-sh/uv:0.12.11`, with multi-platform index digest
`sha256:79c6f4776b851471cc73b7d21d0cc834bb94383c292e83640d27eff512864df7`.

The `latest` image is intentionally distroless and does not contain `/bin/sh`, as discussed in
astral-sh/uv#8635. However, `COPY --from=...` does not execute that source image. I tested the shown
COPY pattern with the current `latest` and `python:3.12-slim`; the copy and a subsequent `uv
--version` both succeeded. astral-sh/uv#21558 also did not change the Docker image layout.

Pinning 0.12.11 or its digest will make the image input reproducible, but it will not fix a build
instruction that runs in the distroless uv stage: that stage has no `/bin/sh` in either 0.12.11 or
0.12.12. To identify this failure, please share the complete Dockerfile, the BuildKit line
identifying the failing step, and the full output for both `latest` and 0.12.11. That will show
which build stage is trying to execute `/bin/sh`.

## Classification

This is a `question`. The issue primarily requests confirmation of a moving tag and pinning
guidance, both of which can be answered from the published releases and image manifests. The
reported failure does not currently establish incorrect uv behavior: the only Dockerfile line
provided follows the documented installation pattern and succeeds with the current image. The
error establishes that an active Docker build stage lacks `/bin/sh`, but the report does not show
which stage or instruction Docker was executing.

The 0.12.12 release timing is confirmed, but no root cause for the reporter's build failure is
confirmed. A complete Dockerfile and full BuildKit output could establish a bug and justify
reclassification. A maintainer has requested that reproduction information, so the present
classification and `needs_more_information` reproduction status remain current. The reporter's
temporary success after rolling back to 0.12.11 raised the possibility of a version-dependent
regression, but the original setup now works again. Together with the repository reproduction
succeeding on both versions, the transient result and missing build context prevent a uv regression
from being established. The maintainer disposition is to take no further action unless the issue is
reopened with a reproduction.

## Related

- astral-sh/uv#8635 — “Can't use uv docker image as a command-line tool” (closed). Maintainers
  explicitly confirm that `ghcr.io/astral-sh/uv:latest` contains only uv binaries and therefore has
  no `/bin/sh`. This is the closest shell-related discussion but is not a duplicate: it concerns
  executing uv inside the distroless container, while astral-sh/uv#21576 uses that image only as a
  `COPY` source.
- astral-sh/uv#21558 — “Bump version to 0.12.12” (merged). This is the release update behind the
  current `latest` tag. Its changed files and the 0.12.12 release notes show a version bump, code
  signing for macOS and Windows artifacts, and an unrelated hash-cutoff fix; it does not change the
  Docker image layout.

## Supporting evidence

- The repository's Docker guide describes `latest` as a distroless image and documents the exact
  `COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/` installation pattern.
- The published `latest` and `0.12.12` tags resolve to the same OCI index and identify version
  0.12.12; the release was published on 2026-09-09. Version 0.12.11 was published on 2026-09-08.
- A clean minimal BuildKit build on Linux using `python:3.12-slim` and the current `latest` completed
  the copy and executed uv successfully. This rules out the supplied snippet as a reproduction but
  does not rule out a problem elsewhere in the reporter's Dockerfile or build environment.
- Repeating the COPY build with 0.12.11 also succeeded. Making either 0.12.12 or 0.12.11 the active
  stage reproduced the shell error, confirming that the shell-free image layout is not new in
  0.12.12 and that the omitted stage selection is material.
- In the reporter's full but undisclosed build, pinning 0.12.11 initially resolved the
  initialization error. The original setup subsequently began working again without an identified
  change, and the reporter can no longer reproduce the failure. They separately report that
  removing `UV_COMPILE_BYTECODE=1` resolved a package-installation timeout. These are user-reported
  observations, not independently reproduced findings, and the relationship between the symptoms
  is unknown.
- A maintainer confirms that the basic uv images have never provided `sh` and raises a possible
  parent-image change. No failing parent-image digest has been supplied, so this remains a
  hypothesis rather than an established cause.
- Recent Docker workflow history contains no image-layout change associated with 0.12.12. The
  nearby workflow changes update artifact-attestation tooling, CI settings, runners, or add Python
  3.15 release-candidate derived images.

## Search coverage

Literal searches covered `stat /bin/sh`, `/bin/sh: no such file or directory`,
`ghcr.io/astral-sh/uv:latest`, the reported `COPY --from` form, `python:3.12-slim`, BuildKit, and
image digests across open and closed issues and open, closed, and merged pull requests. Conceptual
searches covered distroless shell behavior, Docker stage selection, reproducible image pinning,
container publishing, manifest annotations, image releases, and moving `latest` tags. Fix-oriented
checks covered recent merged Docker workflow changes and the 0.12.11-to-0.12.12 release delta.

astral-sh/uv#11961 was inspected but ruled out because it concerns downloading and running an
installer rather than copying binaries. astral-sh/uv#8428 is a registry pull-denial report, and
astral-sh/uv#16350 and astral-sh/uv#15271 concern publishing rate limits and annotations; none match
container initialization or the reported shell error.
