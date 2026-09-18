# Install order is non-deterministic when two distributions claim the same path

Issue: astral-sh/uv#21819

Classification: duplicate

## Summary

The report shows that repeated `uv pip install` operations can produce different bytes when multiple wheels provide the same path with different content. In the supplied reproduction, three Databricks distributions provide `databricks/__init__.py`; the installed file alternates between 537 bytes and zero bytes, changing the digest of an OCI layer built from the virtual environment. The requested capability is a deterministic processing or installation order, without asserting which distribution should be the semantically correct winner.

This is the same underlying install race already tracked in astral-sh/uv#6568. That open issue also involves two distributions providing different contents for one `__init__.py`, repeated Docker builds succeeding or failing according to which content remains, and discussion of defined unpack/install order. astral-sh/uv#13435 is a second open reproduction in which `z3` and `z3-solver` overwrite the same module and identical `uv pip` runs produce different environments.

The current source supports the reported ordering concern: preparation uses `FuturesUnordered`, collects prepared wheels in completion order after an unstable size sort, and installation processes the resulting wheel vector with Rayon. Setting `UV_CONCURRENT_INSTALLS=1` limits the Rayon pool but does not restore the original distribution order. This establishes a correctness problem, but the open, directly matching astral-sh/uv#6568 makes `duplicate` the appropriate classification.

## Draft response

Thanks for the detailed reproduction. This is the same underlying install race tracked in astral-sh/uv#6568: when distributions write different contents to the same path in one operation, the resulting file can depend on processing and write order, so identical installs are not deterministic. astral-sh/uv#13435 contains another confirmed reproduction, while astral-sh/uv#13437 added conflict detection rather than deterministic arbitration. We’ll centralize this in astral-sh/uv#6568, so I’m closing this as a duplicate.

## Classification

`duplicate` takes precedence because astral-sh/uv#6568 is open and tracks the same underlying behavior closely enough to centralize discussion: `uv pip`, multiple distributions writing different contents to the same `__init__.py`, repeated container builds producing different outcomes, and a request to control or define installation order. astral-sh/uv#13435 independently confirms the same race and explicitly includes a request for deterministic ordering.

Absent those existing trackers, the report would be a bug rather than an enhancement: identical inputs producing byte-different environments is incorrect behavior, and the current source confirms an order-losing preparation path followed by parallel installation. The report does not require a reproduction request before classification because it already includes a minimal command sequence, exact package versions, the differing file sizes, and measured repeated-run behavior.

## Related

- astral-sh/uv#6568 — **Race condition when two libraries update the same __init__.py** (open issue). This is the primary duplicate. It reports repeated `uv pip` Docker installs leaving different `jwt/__init__.py` contents depending on which distribution overwrites the file last. The discussion explicitly considers defined unpack order, notes that the operation is not deterministic, and requests mitigation for this race.
- astral-sh/uv#13435 — **Nondeterministic behavior of `uv pip`** (open issue). This is another direct symptom and mechanism match: `z3` and `z3-solver` provide overlapping module files, and parallel installation produces different contents and success/failure outcomes across identical runs. A later commenter specifically asks for deterministic install order.
- astral-sh/uv#13437 — **Warn when two packages write to the same module** (merged pull request). This added detection for overlapping top-level modules after several reports of packages working only intermittently. It documents the same non-deterministic install race but provides a warning, not deterministic arbitration. The detection was subsequently gated as a preview feature and refined.
- astral-sh/uv#15813 — **Better support for conflicting transitive dependencies** (closed issue). This reports random results when concurrently installing distributions that share module namespaces. Maintainer discussion explicitly states that choosing an arbitrary package across repeated `uv run` or `uv sync` operations is still broken and that deterministic order matters. Its reporter focused on preventing hybrid modules rather than choosing a stable same-file winner, so astral-sh/uv#6568 is the better canonical duplicate.

## Search coverage and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries included “same path,” “install order,” “nondeterministic install,” “non-deterministic install,” “deterministic order,” Databricks plus `__init__.py`, `UV_CONCURRENT_INSTALLS`, `FuturesUnordered`, `sort_unstable_by_key`, and the preview feature identifier. Conceptual queries included overlapping or conflicting files/modules, packages overriding one another, shared paths and namespaces, install race conditions, parallel wheel installation, reproducible installs, and OCI or virtual-environment digests. Fix-oriented searches covered deterministic installer/wheel ordering, conflict warnings, and overlapping-file changes.

Exact internal identifiers and the Databricks package combination appeared only in astral-sh/uv#21819. The observable symptom searches found astral-sh/uv#6568 and astral-sh/uv#13435; repository vocabulary around “install race condition” and “module conflicts” led to astral-sh/uv#13437, astral-sh/uv#15357, astral-sh/uv#15813, and astral-sh/uv#16546. The strongest candidates and their maintainer comments and referenced chains were inspected.

astral-sh/uv#15357 and astral-sh/uv#16546 are plausible adjacent results but are not the canonical duplicate: they track stabilization of the preview conflict warning and an optional error for package overrides, respectively, rather than deterministic file contents. astral-sh/uv#15238 and astral-sh/uv#17645 concern uninstalling one overlapping distribution and thereby removing files still claimed by another, which is a distinct transition-time corruption mechanism. astral-sh/uv#20996 demonstrates an overlapping bundled module but was handled as an external packaging problem rather than a repeated-run determinism report.
