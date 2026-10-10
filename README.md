# All py315 Docker tags still reference python3.15-rc rather than python3.15?

Issue: astral-sh/uv#22459

Classification: bug (Docker stable-image transition / publication)

## Summary

The reported RC Docker tags are reproducible through anonymous registry queries on October 10, 2026, at approximately 14:15–14:16 UTC. GHCR lists 92 tags containing `python3.15`, all with `python3.15-rc-`; no stable Python 3.15 aliases are listed. The current Alpine, Trixie, and slim-Trixie images declare `PYTHON_VERSION=3.15.0rc3` on both Linux amd64 and arm64, with uv version labels identifying release 0.13.0. The corresponding stable upstream Python image tags return `manifest unknown`.

The issue was opened on October 10, 2026, at 14:10 UTC. The reporter expected stable Python 3.15 images after Python's final release and observed RC tags despite an update approximately 18 hours earlier. No exact image reference, digest, command output, platform, or uv version was supplied. The reproduction uses the public aliases documented by uv, without assuming a project configuration or a CLI upgrade regression.

astral-sh/uv#22421 promoted the Python 3.15 Docker tags, but astral-sh/uv#22435 deliberately reverted them because upstream stable images were unavailable. The current registry results corroborate that publication dependency. Reproducing the RC references does not establish that retaining them while upstream stable tags are unavailable is incorrect.

## Classification

The bug classification tracks the incomplete stable Docker-image transition. The stable mappings were merged and then rolled back; there is no evidence that stable uv Python 3.15 Docker images were successfully published before the rollback. This is a known Docker publication gap with an intentional temporary RC configuration, not evidence of a uv CLI resolver or managed-Python installation failure.

astral-sh/uv#22438 addresses the CI validation gap that allowed the promotion to merge before Docker builds completed. It does not restore stable tags. The related managed-Python catalog changes are a separate release path. No open canonical tracker for the Docker rollback was found in the related-issue search.

## Reproduction

**Outcome: reproducible.** The reported tag naming and RC image metadata were observed directly in public registries, independently of source inspection.

### Environment and isolation

- Host: Ubuntu 24.04.5 LTS, Linux x86_64.
- Registry client: Skopeo 1.13.3, anonymous access with `--no-creds` and a newly created empty authentication file.
- Installed executable on PATH: `uv 0.12.13 (x86_64-unknown-linux-gnu)`. The inspected images identify uv 0.13.0 through their OCI version labels; the host uv version does not control registry tag selection.
- Image configurations checked for Linux amd64 and arm64. No Python executable was run inside an image.
- Files and cache/config directories are under `/home/runner/work/_temp/issue-22459-repro.XpVGwE`. No checkout files, existing Docker state, or GitHub state were changed.

### Minimal commands

The following reconstructs the anonymous metadata queries, using only a temporary directory:

```sh
repro_dir=$(mktemp -d)
mkdir -p "$repro_dir/docker-config" "$repro_dir/cache"
printf '%s\n' '{"auths":{}}' > "$repro_dir/auth.json"
export DOCKER_CONFIG="$repro_dir/docker-config"
export XDG_CACHE_HOME="$repro_dir/cache"

skopeo list-tags --no-creds --authfile "$repro_dir/auth.json" \
  docker://ghcr.io/astral-sh/uv > "$repro_dir/uv-tags.json"
jq '[.Tags[] | select(contains("python3.15"))]' "$repro_dir/uv-tags.json"

skopeo --override-os linux --override-arch amd64 inspect \
  --no-creds --authfile "$repro_dir/auth.json" --config \
  docker://ghcr.io/astral-sh/uv:python3.15-rc-trixie-slim

skopeo inspect --no-creds --authfile "$repro_dir/auth.json" --raw \
  docker://ghcr.io/astral-sh/uv:python3.15-trixie-slim

skopeo inspect --no-creds --authfile "$repro_dir/auth.json" --raw \
  docker://docker.io/library/python:3.15-slim-trixie
```

The RC inspection succeeds and returns `PYTHON_VERSION=3.15.0rc3` and `org.opencontainers.image.version=0.13.0-python3.15-rc-trixie-slim`. Both stable-tag inspections exit 1 with `manifest unknown`.

### Registry observations

All 92 GHCR tags containing `python3.15` contain `python3.15-rc-`, including the four floating aliases and their `0.13.0-` release aliases. The floating aliases are:

- `ghcr.io/astral-sh/uv:python3.15-rc-alpine`
- `ghcr.io/astral-sh/uv:python3.15-rc-alpine3.23`
- `ghcr.io/astral-sh/uv:python3.15-rc-trixie`
- `ghcr.io/astral-sh/uv:python3.15-rc-trixie-slim`

Additional `skopeo inspect --no-tags` queries selected both amd64 and arm64 for each of the three image families. Every selected configuration declares Python 3.15.0rc3 and a uv 0.13.0 RC-image version label. Skopeo reported these image-index digests:

| GHCR tag | Index digest |
| --- | --- |
| `python3.15-rc-alpine3.23` | `sha256:6d28515c370ead51beb640ef25579c8158f7bf7da68908f98b1e506176768e38` |
| `python3.15-rc-trixie` | `sha256:5de4387bea659ee2a481fd756c717c59d17b25f80b046ca3e27dc91bb8d8d227` |
| `python3.15-rc-trixie-slim` | `sha256:282744aba5895f0ead53afb96c5af9be462000926d3d787a031d88405e523abb` |

The Alpine alias resolves to the same digest as Alpine 3.23. The Docker Hub mirror `docker.io/astral/uv:python3.15-rc-trixie-slim` also resolves to the same slim-Trixie digest and declares the same Python version on amd64. The other Docker Hub aliases were not inspected.

All three upstream stable references return exit 1 with `manifest unknown`:

- `docker.io/library/python:3.15-alpine3.23`
- `docker.io/library/python:3.15-trixie`
- `docker.io/library/python:3.15-slim-trixie`

Image configuration creation timestamps fall around October 9, 2026, at 19:20 UTC, consistent with a recent image update. These timestamps do not independently establish the exact registry push time or the reporter's “18 hours” display.

The local artifacts include `uv-tags.json`, `uv-rc-slim-config.json`, and `registry-results.json`; `inspect-images.py` records the explicit references and architecture selections used for the additional checks.

### Existing coverage and limits

Searches of `crates/uv/tests/`, prioritizing `crates/uv/tests/it/`, and `crates/uv-client/tests/it/` found no integration test asserting these published Docker aliases or upstream base-image availability. Python installation tests concern managed interpreter downloads, a separate path. No Rust test suite was run or changed for this registry observation. The relevant CI coverage is the Docker build workflow, now included in required dependencies by astral-sh/uv#22438.

This reproduces the published RC tags and their declared Python version. Image layers were not downloaded or executed, so it is not a runtime `python --version` check. Historical interpreter contents behind every versioned tag were not inspected. No last-known-good stable Python 3.15 Docker image was supplied or established; comparing host uv CLI versions would not test this publication behavior.

## Related

- astral-sh/uv#22421 — Promote Python 3.15 Docker images to stable tags (merged pull request). Merged on October 9, 2026, replacing all three Python 3.15 RC base-image mappings and four documented tags with stable equivalents. Its changes were subsequently reversed, so it is the historical fix for the reported regression, not a completed resolution.
- astral-sh/uv#22435 — Restore Python 3.15 release-candidate Docker images (merged pull request). Merged about 25 minutes after astral-sh/uv#22421 and reversed its stable mappings and documentation. The PR explains that upstream stable base images were unavailable and Docker builds could not resolve them; this rollback directly explains the current RC references.
- astral-sh/uv#22438 — Require planned Docker builds to pass (merged pull request). Explicitly cites astral-sh/uv#22421 as an image change that merged before its Docker builds finished. Adds Docker builds to the required CI dependencies; it addresses the validation gap, not restoration of stable Python 3.15 tags.
- astral-sh/uv#22439 — Make CPython 3.15.0 final available through the released uv managed-Python catalog (closed issue). Concerns the same Python final-release timing, but reports missing downloads for uv python install. Its linked catalog update, astral-sh/uv#22400, does not change Docker mappings, so this is adjacent context rather than a duplicate or Docker fix.

## Supporting evidence

- `.github/workflows/build-docker.yml:113` defines entries as `base-image,tag1,tag2,...`. Lines 121, 128, and 135 select `python:3.15-rc-alpine3.23`, `python:3.15-rc-trixie`, and `python:3.15-rc-slim-trixie`, with matching RC tags for uv. The generated derived Dockerfile uses the selected Python base and copies the uv executables into it.
- `docs/guides/integration/docker.md:66` and the adjacent image lists document the four RC aliases observed in GHCR.
- Public GitHub API inspection confirms that astral-sh/uv#22421 merged on October 9 at 18:02:53 UTC and removed `-rc` from the three matrix entries and four documented aliases. astral-sh/uv#22435 merged at 18:28:11 UTC and reversed those changes, explicitly documenting unavailable upstream stable base images. These diffs establish the reason for the configured rollback; the registry checks independently establish the current published behavior.
- astral-sh/uv#22438 merged at 18:32:45 UTC and adds `build-docker` to required CI dependencies. It does not change image mappings.
- `CHANGELOG.md` for uv 0.13.0, released October 9, describes Python 3.15 as the default stable managed-Python version and cites astral-sh/uv#22400 for CPython 3.15.0. That catalog change does not replace the Docker base images.
- Upstream `docker-library/python` source metadata inspected during issue classification listed `3.15-rc` with version `3.15.0rc3` and no `3.15` entry. Direct registry checks now confirm the absence of the three required stable upstream tags at reproduction time.

## Draft response

The published Python 3.15 Docker tags still use RC names. Registry checks show that the Alpine, Trixie, and slim-Trixie images declare Python 3.15.0rc3 on both supported architectures. astral-sh/uv#22421 switched the mappings to stable tags, but astral-sh/uv#22435 temporarily restored RC tags because the upstream stable Python images were unavailable. Those three upstream stable tags still return `manifest unknown` in the reproduction. Python's final release, uv's managed-Python catalog, and publication of Docker base images are separate steps; the recent uv image update did not complete the Docker transition.

## Search scope and excluded candidates

Used authenticated gh to search open/closed issues and open/closed/merged PRs for py315, python3.15-rc, 3.15 with Docker, stable tags, RC/prerelease/release-candidate images, base images, and internal:releases. Broadened to version-independent Docker titles and historical Python 3.14/3.13 image transitions. PR text searches returned no matches, so supplemented them with paginated open PRs, recent all-state PRs, workflow commit history, and direct inspection of candidate bodies, comments, reviews, diffs, and cross-reference timelines. No open tracker for the rollback was found. Ruled out astral-sh/uv#22439 and astral-sh/uv#22400 as managed-Python catalog work, astral-sh/uv#8954 as uv prerelease-version syntax, astral-sh/uv#16652 as floating latest-version aliases, and astral-sh/uv#21597 as shared-action refactoring that retains the image matrix.

The Docker history also led to astral-sh/uv#21293, which introduced the Python 3.15 RC variants, and its predecessor astral-sh/uv#13390, which added Python 3.14 prerelease images. Comments on the latter establish that RC naming mirrors upstream images and avoids presenting prereleases as stable. The older astral-sh/uv#8066 discussion and its closing pull request, astral-sh/uv#8105, concern adding Python 3.13 Docker variants, not the current rollback.

astral-sh/uv#16652 requests aliases that follow the latest Python and Debian versions; it does not track promotion of a fixed Python minor version. astral-sh/uv#8954 concerns incompatible uv prerelease-version spellings in a tag consistency check. The open astral-sh/uv#21597 explicitly retains uv's image matrix while adopting shared Docker actions. None supplies an open canonical discussion for this regression.

## Maintainer next step

Once upstream `python:3.15-alpine3.23`, `python:3.15-trixie`, and `python:3.15-slim-trixie` become available, verify both configured architectures, restore the stable mappings and documentation, require the planned Docker builds to pass, and verify the resulting uv tags in the public registries. Current evidence supports the temporary rollback explanation; it does not show that stable images were published and subsequently regressed at runtime.
