# Remove misleading note from uv documentation

Issue: astral-sh/uv#23065

Classification: bug

## Summary

The reporter requests removal of the note under “Using the uv image” in the GitLab integration guide. It shows a distroless image with `entrypoint: [""]`, implying that resetting the entrypoint makes the image suitable for GitLab CI. The report cites GitLab's shell requirement and astral-sh/uv#9361. No platform or uv version was supplied.

The current source confirms the documentation error: the image contains only the uv binaries, and resetting an entrypoint cannot add the shell needed to run a GitLab job. astral-sh/uv#9093 introduced the misleading note; astral-sh/uv#9361 confirms that clearing the entrypoint still leaves GitLab without a shell. astral-sh/uv#7054 applies only to derived images.

## Draft response

You're right: clearing the entrypoint does not make the distroless image usable as a GitLab CI job image, because it contains only uv and uvx and has no shell. astral-sh/uv#9361 confirms the failure, and the discussion in astral-sh/uv#9093 states the same limitation. The note should be removed or replaced with an explicit requirement to use a non-distroless image. For your jobs, use the Python-based image shown immediately above the note.

## Classification

The guide incorrectly implies that clearing the entrypoint makes a distroless image usable by GitLab. Dockerfile builds that image FROM scratch with only uv and uvx, and historical reports confirm the missing-shell failure. This establishes a documentation correctness bug despite the question framing. astral-sh/uv#9361 resolved image-selection support before the note existed; astral-sh/uv#9093 introduced the note rather than fixing it. Neither tracks the requested correction, and no prior fix of this documentation defect was found.

This is not a request to add a shell to uv's distroless images. The actionable defect is the GitLab guide's advice. A fresh reproduction is not needed to establish that the published example conflicts with the image contents and the documented historical failure.

## Related

- astral-sh/uv#9093 — Docs(GitLab): add entrypoint for distroless image (merged pull request). Added the exact misleading note on December 4, 2024. Its discussion explicitly states that GitLab cannot use the distroless image because it lacks sh. This introduced the documentation defect; it did not correct it.
- astral-sh/uv#9361 — UV on gitlab CICD has some issue (closed issue). Shows GitLab failing with a missing sh executable even after entrypoint: [""] was configured. A maintainer recommended a non-distroless image, which resolved the reporter's problem. Closed before the misleading note was added, so it corroborates the failure without tracking this documentation correction.
- astral-sh/uv#7054 — ci(docker)!: adjust entrypoint and cmd for inherited images (merged pull request). Referenced by astral-sh/uv#9093; removed the custom entrypoint from derived images only. It neither supplied a shell in distroless images nor fixed the GitLab note, so this is not a regression of that change.

## Supporting evidence

- `docs/guides/integration/gitlab.md:29` contains the note and its distroless image example. The main example at line 24 already uses a Python-based derived image.
- `Dockerfile:70` starts the final image with `FROM scratch`, copies only `/uv` and `/uvx`, and sets `ENTRYPOINT ["/uv"]`. It does not include `sh`.
- `docs/guides/integration/docker.md:17` states that distroless images contain only the uv binaries, while derived images include an operating system.
- In astral-sh/uv#9361, the Docker executor initially reports `unrecognized subcommand 'sh'`. After the reporter clears the entrypoint, it instead reports `exec: "sh": executable file not found in $PATH`. On November 22, 2024, a maintainer recommends a non-distroless image and the reporter confirms that changing the image resolves the problem.
- The November 21, 2024 discussion in astral-sh/uv#9093 explicitly says the distroless image is unsupported in GitLab because `sh` does not exist. The author also acknowledges having changed images between tests. The merged patch nevertheless adds the exact note now reported as misleading.
- The chronology distinguishes this defect from a regression: astral-sh/uv#7054 changed derived-image entrypoints on September 4, 2024; astral-sh/uv#9361 closed on November 22; astral-sh/uv#9093 added the note on December 4. No subsequent correction to the note was identified in the guide's history.
- Following the reference chain also found astral-sh/uv-docker-example#31, the same reporter's earlier support report. A maintainer there explains that the scratch image has no shell and recommends a derived image; the reporter redirects the discussion to astral-sh/uv#9361.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using GitLab, distroless, entrypoint, shell, the guide anchor, exact missing-executable and unrecognized-subcommand fragments, documentation terminology, and referenced issue numbers. Inspected comments, patches, reference chains, and the GitLab guide's commit history. PR keyword searches returned no results despite known matching merged PRs, so supplemented them with direct PR retrieval and all open PR listings. Ruled out astral-sh/uv#8635 (libc/Python discovery), astral-sh/uv#16432 (publishing documentation), and astral-sh/uv#22542 (unrelated wording corrections). No existing correction was identified.

Literal searches covered the report's image type, entrypoint setting, guide anchor, and referenced issue. Conceptual searches covered GitLab CI runners, shell availability, derived/non-distroless images, and documentation feedback, including comments. Fix-oriented checks covered closed issues, merged PRs, the derived-image entrypoint change, and the full GitLab guide commit history obtained through GitHub because the local checkout is shallow.

astral-sh/uv#8635 concerns libc detection and managed-Python discovery after uv starts, rather than GitLab failing to start a shell. astral-sh/uv#16432 adds a publishing/release example to the same guide and does not change image-selection advice. astral-sh/uv#22542 only fixes wording in the guide's system-installation section. The relevant comments inspected in the general documentation tracker, astral-sh/uv#5605, do not track this note.

PR keyword search results were incomplete: known relevant merged PRs were absent. Directly inspecting linked PRs, the guide's history, and all 650 currently open PR titles and bodies supplied additional coverage. The reporter's external GitLab documentation URL returned HTTP 403 in this environment; the conclusion rests on repository source, historical failure output, and maintainer comments.

## Recommended next step

Remove the distroless entrypoint example, or replace it with a short explanation that GitLab job images need a shell and should use a suitable derived image. Keep the existing Python-based example as the supported path. The proposed scope is a documentation correction; no runtime change is needed.

This handoff is based on read-only repository and GitHub inspection. No build or test was run, no checkout file was modified, and no GitHub change was made.
