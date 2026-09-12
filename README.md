# Support org formatted README by default

Issue: astral-sh/uv#21635

Classification: enhancement

## Summary

The reporter wants `README.org` to be recognized by extension wherever uv derives Python package
README metadata, instead of producing an unknown-extension error. The report does not include a
command, complete error, uv version, build-backend declaration, or reproduction.

The current `uv-build-backend` implementation infers `text/markdown` for `.md`, `text/x-rst` for
`.rst`, and `text/plain` for `.txt`. It rejects other extensions and limits explicitly configured
README content types to those same three values. This makes Org support a request to expand the
backend's supported formats, not a failure to implement its current supported-extension behavior.

The only exact historical report is astral-sh/uv#9821. In that case the same `README.org` error came
from Hatchling, and a maintainer closed it as outside uv. No existing uv issue or pull request was
found that tracks Org support in `uv-build-backend`.

## Draft response

uv-build-backend currently infers README metadata only for `.md`, `.rst`, and `.txt` files,
corresponding to the content types it can emit for package metadata. GitHub's repository README
rendering support does not by itself add an Org content type to Python package metadata, so
`README.org` cannot currently be inferred in the same way as `README.md`.

We'll treat this as a request to expand the build backend's supported README formats. In the
meantime, if plain-text package metadata is acceptable, use the explicit form:

```toml
readme = { file = "README.org", content-type = "text/plain" }
```

## Classification

This is an enhancement because it asks uv to infer and support an additional README format by
default. Repository source confirms that the current validation is deliberate: extension inference
and explicit content types are restricted to Markdown, reStructuredText, and plain text, and the
error tells users to choose a supported extension or set a content type manually.

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
explicitly. This supports the classification and the plain-text workaround in the draft response.
