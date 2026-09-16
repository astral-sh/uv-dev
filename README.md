# Support org formatted README by default

Issue: astral-sh/uv#21635

Classification: enhancement

## Summary

The reporter wants `README.org` to be recognized by extension when uv derives Python package README
metadata. A follow-up reproduces the failure with `uv sync` under Python 3.14.0: the build fails as
invalid project metadata because uv cannot infer a content type from the `.org` extension. The uv
version, platform, and complete build-backend configuration remain unspecified.

The current `uv-build-backend` implementation infers `text/markdown` for `.md`, `text/x-rst` for
`.rst`, and `text/plain` for `.txt`. It rejects other extensions and limits explicitly configured
README content types to those same three values. This makes Org support a request to expand the
backend's supported formats, not a failure to implement its current supported-extension behavior.
The follow-up further establishes that explicitly setting `text/org` fails as unsupported. Proper
Org rendering therefore depends on adding an Org content type to the Python core metadata
specification before uv can emit it.

The only exact historical report is astral-sh/uv#9821. In that case the same `README.org` error came
from Hatchling, and a maintainer closed it as outside uv. No existing uv issue or pull request was
found that tracks Org support in `uv-build-backend`.

## Reproduction and observed behavior

The follow-up reports these two configurations and outcomes:

1. With `project.readme` referring to `README.org`, run `uv sync`:

   ```text
   Failed to build `test`
   Invalid project metadata
   Unknown Readme extension `org`, can't determine content type. Please use a support extension (`.md`, `.rst`, `.txt`) or set the content type manually.
   ```

2. Set the README's content type explicitly to `text/org` and run `uv sync` again:

   ```text
   Failed to build `test`
   Invalid project metadata
   Unsupported content type: text/org
   ```

The first run used CPython 3.14.0. No uv version or platform was provided. The follow-up rejects
declaring Org content as `text/plain` because that would misrepresent its format, so the previously
suggested plain-text workaround is not an adequate resolution for this request.

## Classification

This is an enhancement because it asks uv to infer and support an additional README format by
default. Repository source and the reported reproduction agree that extension inference and
explicit content types are restricted to Markdown, reStructuredText, and plain text. The follow-up
identifies the governing Python core metadata specification as the limiting layer: it defines only
`text/plain`, `text/x-rst`, and `text/markdown` for `Description-Content-Type`. Supporting Org as
Org, rather than treating it as plain text, first requires specification-level support for an Org
content type.

This is not a duplicate of astral-sh/uv#9821. Although that issue has the same filename and
observable failure, its traceback identified Hatchling as the component rejecting the extension,
and the maintainer closed it on that basis. Since `uv-build-backend` now contains its own analogous
validation, the new request is appropriately tracked as a uv build-backend enhancement.

The new issue already carries the repository's `enhancement` label, consistent with this
classification.

## Related

- astral-sh/uv#9821 (closed), “Building fails if you use README.org vs README.md” — This is the exact
  earlier symptom and filename. Its error was raised by Hatchling, however, and a uv maintainer
  explicitly closed it as not a uv issue. It is useful precedent but not the canonical tracker for
  adding Org support to `uv-build-backend`.

## Search and supporting evidence

Literal searches covered `README.org`, Org README, org-mode, and Org-format wording across open and
closed issues and open, closed, and merged pull requests. Conceptual searches covered README
extension inference, unknown-extension errors, README content types, `Description-Content-Type`,
PEP 621, `uv_build`, build-backend metadata, and explicit README tables. Fix-oriented searches
included closed issues, merged pull requests, the build-backend issue inventory, and recent pull
request bodies. No pull request implementing Org support was found.

Two especially plausible adjacent results were inspected and excluded from the related list:

- astral-sh/uv#14761 and merged astral-sh/uv#14762 concern a TOML parsing bug in the explicit
  `{ file, content-type }` README form. They fixed support for that already-defined configuration
  shape, not extension inference or an Org content type.
- astral-sh/uv#8779 is the completed build-backend tracking issue. It is too broad to centralize
  this request and explicitly directs specific feature requests to separate issues.

In `crates/uv-build-backend/src/metadata.rs`, `PyProjectToml::to_metadata` maps only `.txt`, `.rst`,
and `.md`, and `ValidationError::UnknownExtension` documents that supported set. The same function
accepts only `text/plain`, `text/x-rst`, and `text/markdown` when a content type is supplied
explicitly. The reported `uv sync` failures match both validation branches.
