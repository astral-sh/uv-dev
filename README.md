# ModuleNotFound when run 'uv run <project_name>' in mac

Issue: astral-sh/uv#20945
Classification: duplicate

## Summary

Open astral-sh/uv#9902 is the closest behavioral duplicate, while open astral-sh/uv#16977 tracks the same symptom and the confirmed hidden-`.pth` mechanism. No related fixing pull request was found.

## Classification

The observable behavior is already tracked closely by open astral-sh/uv#9902 and astral-sh/uv#16977: a macOS editable project becomes unimportable while uv regards it as installed, and forcing freshness invalidation temporarily restores it. The exact external trigger for astral-sh/uv#20945 remains unconfirmed, but the matching command, error path, installation-state transition, platform, and workaround are sufficient to centralize investigation in those existing threads.

## Related

- https://github.com/astral-sh/uv/issues/9902 (open issue): Bug: ModuleNotFoundError related to venv uv_cache.json when store in iCloud
  astral-sh/uv#9902 has the same macOS `uv init --package`/`uv run` failure: the console script intermittently raises `ModuleNotFoundError` while uv reports the editable project as already installed. Invalidating `uv_cache.json` changes the result to “installed, but not fresh” and reinstalls successfully, closely matching the reported `pyproject.toml` timestamp workaround. That thread confirms iCloud-synced directories as its trigger.
- https://github.com/astral-sh/uv/issues/16977 (open issue): Local packages can become unimportable due to skipped hidden `.pth` files
  astral-sh/uv#16977 covers the same macOS/Python 3.12/src-layout symptom while uv considers the editable package installed. Its thread identifies Python skipping the editable `.pth` after macOS marks it `UF_HIDDEN`; comments connect the delayed flag change plausibly to iCloud. This directly matches the new report’s stated `.pth` failure, although that mechanism has not yet been independently demonstrated for the new reproduction.
