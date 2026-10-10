# All py315 Docker tags still reference python3.15-rc rather than python3.15?

Issue: astral-sh/uv#22459

Classification: bug

## Summary

The reporter observes that all Python 3.15 Docker tags still contain `python3.15-rc` after Python 3.15's expected stable release, despite an image update approximately 18 hours earlier. The issue was opened on October 10, 2026, at 14:10 UTC. No exact image digest, command output, platform, or uv version was supplied.

astral-sh/uv#22421 promoted Python 3.15 Docker tags, but astral-sh/uv#22435 reverted them because upstream stable images were unavailable. astral-sh/uv#22438 fixes the CI validation gap; astral-sh/uv#22439 concerns a separate managed-Python release path.

The current checkout confirms the RC references in both the build matrix and the Docker guide. The stable-tag transition was merged and then explicitly rolled back; its prior merge does not establish that stable images were successfully published.

## Draft response

You're seeing the temporary rollback of the stable-tag update. astral-sh/uv#22421 switched the Python 3.15 Docker variants to stable tags, but astral-sh/uv#22435 restored the RC tags because the upstream stable base images could not be resolved. The current build matrix still uses those RC mappings, and upstream Docker metadata still lists 3.15.0rc3. Python's final release and publication of its Docker images are separate steps. The next step is to verify availability of the stable Alpine, Trixie, and slim-Trixie base images, then restore the stable mappings and publish updated uv images.

## Classification

The stable-tag transition was implemented by astral-sh/uv#22421 and then reversed by astral-sh/uv#22435; the checkout confirms that all Python 3.15 Docker mappings and documented tags again contain -rc. This is a source-level regression, not a duplicate of the merged promotion. The rollback had a documented upstream-availability reason, and upstream metadata still lists 3.15.0rc3; restoring stable images remains dependent on upstream publication. There is no evidence that stable uv Docker tags were successfully published before the rollback, and no open tracker for this regression was found.

The report's question wording does not change the classification of the reverted stable-tag transition. The upstream dependency explains why the rollback was necessary; it does not mean the stable-image transition is complete. This handoff distinguishes the confirmed configuration regression from any unverified claim about a published image's runtime Python version.

## Related

- astral-sh/uv#22421 — Promote Python 3.15 Docker images to stable tags (merged pull request). Merged on October 9, 2026, replacing all three Python 3.15 RC base-image mappings and four documented tags with stable equivalents. Its changes were subsequently reversed, so it is the historical fix for the reported regression, not a completed resolution.
- astral-sh/uv#22435 — Restore Python 3.15 release-candidate Docker images (merged pull request). Merged about 25 minutes after astral-sh/uv#22421 and reversed its stable mappings and documentation. The PR explains that upstream stable base images were unavailable and Docker builds could not resolve them; this rollback directly explains the current RC references.
- astral-sh/uv#22438 — Require planned Docker builds to pass (merged pull request). Explicitly cites astral-sh/uv#22421 as an image change that merged before its Docker builds finished. Adds Docker builds to the required CI dependencies; it addresses the validation gap, not restoration of stable Python 3.15 tags.
- astral-sh/uv#22439 — Make CPython 3.15.0 final available through the released uv managed-Python catalog (closed issue). Concerns the same Python final-release timing, but reports missing downloads for uv python install. Its linked catalog update, astral-sh/uv#22400, does not change Docker mappings, so this is adjacent context rather than a duplicate or Docker fix.

## Supporting evidence

- `.github/workflows/build-docker.yml:113` defines matrix entries as `base-image,tag1,tag2,...`. Lines 121, 128, and 135 explicitly select `python:3.15-rc-alpine3.23`, `python:3.15-rc-trixie`, and `python:3.15-rc-slim-trixie`, with matching RC tags for uv. A subsequent build using these entries will retain the RC names.
- `docs/guides/integration/docker.md:66`, line 67, line 81, and line 89 document the four matching Python 3.15 RC aliases. The report is consistent with both configuration and documentation.
- astral-sh/uv#22421 merged on October 9 at 18:02:53 UTC. Its diff removes `-rc` from all three matrix entries and four documented aliases.
- astral-sh/uv#22435 merged at 18:28:11 UTC on the same day. Its diff reverses those changes. Its description explicitly says that stable upstream images were not available, Docker builds could not resolve them, and the restoration of RC images was temporary. Both PRs have no comments or reviews adding a different explanation.
- astral-sh/uv#22438 merged at 18:32:45 UTC. Its description connects the premature merge of astral-sh/uv#22421 to omitted Docker builds in the required CI dependency list. Its only changed file is `.github/workflows/ci.yml`; it does not restore stable Docker mappings.
- The public `docker-library/python` repository's `versions.json`, inspected during this triage, contains a `3.15-rc` entry with version `3.15.0rc3` and no `3.15` entry. This supports the upstream publication dependency but is not a registry manifest or pull test.
- astral-sh/uv#22400 merged stable CPython 3.15.0 into the managed-Python catalog on October 9 at 17:34:16 UTC. Its changed files concern download metadata, sysconfig mappings, and tests, not Docker image configuration. The maintainer's “We're releasing this as quickly as possible” comment on astral-sh/uv#22439 concerns that catalog release, not a promise about Docker tags.

## Search scope and excluded candidates

Used authenticated gh to search open/closed issues and open/closed/merged PRs for py315, python3.15-rc, 3.15 with Docker, stable tags, RC/prerelease/release-candidate images, base images, and internal:releases. Broadened to version-independent Docker titles and historical Python 3.14/3.13 image transitions. PR text searches returned no matches, so supplemented them with paginated open PRs, recent all-state PRs, workflow commit history, and direct inspection of candidate bodies, comments, reviews, diffs, and cross-reference timelines. No open tracker for the rollback was found. Ruled out astral-sh/uv#22439 and astral-sh/uv#22400 as managed-Python catalog work, astral-sh/uv#8954 as uv prerelease-version syntax, astral-sh/uv#16652 as floating latest-version aliases, and astral-sh/uv#21597 as shared-action refactoring that retains the image matrix.

The Docker history also led to astral-sh/uv#21293, which introduced the Python 3.15 RC variants, and its predecessor astral-sh/uv#13390, which added Python 3.14 prerelease images. Comments on the latter establish that RC naming mirrors upstream images and avoids presenting prereleases as stable. The older astral-sh/uv#8066 discussion and its closing pull request, astral-sh/uv#8105, concern adding Python 3.13 Docker variants, not the current rollback.

astral-sh/uv#16652 requests aliases that follow the latest Python and Debian versions; it does not track promotion of a fixed Python minor version. astral-sh/uv#8954 concerns incompatible uv prerelease-version spellings in a tag consistency check. The open astral-sh/uv#21597 explicitly retains uv's image matrix while adopting shared Docker actions. None supplies an open canonical discussion for this regression.

## Maintainer next step and verification limits

Verify that stable upstream `python:3.15-alpine3.23`, `python:3.15-trixie`, and `python:3.15-slim-trixie` images are available for both configured architectures before restoring the stable mappings and documentation. Require the planned Docker builds to pass and verify publication of the resulting uv tags.

The findings come from repository configuration, PR diffs and discussion, workflow history, and upstream source metadata. No Docker image was pulled or executed, so the reporter's “18 hours” observation and the interpreter version inside a particular published digest remain unverified. No source change or test run was needed for this triage.
