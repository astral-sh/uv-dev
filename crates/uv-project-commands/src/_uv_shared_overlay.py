"""Add shared dependencies after the writable environment's editable paths."""

import site

_pending = []
_original_execsitecustomize = None


def add_overlay(path):
    global _original_execsitecustomize
    # Dynamic addsitedir calls have no remaining interpreter startup hook.
    if not getattr(site.__spec__, "_initializing", False):
        site.addsitedir(path)
        return
    # A virtual environment may process its .pth files more than once.
    # Keep parent precedence stable without executing their .pth files repeatedly.
    if path not in _pending:
        _pending.append(path)
    # site runs this hook after all startup .pth files, before sitecustomize.
    if _original_execsitecustomize is None:
        _original_execsitecustomize = site.execsitecustomize
        site.execsitecustomize = _extend_site


def _extend_site():
    global _original_execsitecustomize
    original = _original_execsitecustomize
    try:
        for path in _pending:
            site.addsitedir(path)
    finally:
        site.execsitecustomize = original
        _original_execsitecustomize = None
        _pending.clear()
    original()
