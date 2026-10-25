"""Keep shared dependencies available behind writable environment paths."""

import site
import sys

_parents = []
_parent_paths = []
_adding_parent = 0
_finished = False
_original_addpackage = None
_original_execsitecustomize = None


def add_overlay(path):
    global _adding_parent, _original_addpackage, _original_execsitecustomize
    # Explicit addsitedir calls after startup keep the normal immediate behavior.
    if _finished or not getattr(site.__spec__, "_initializing", False):
        site.addsitedir(path)
        return
    # A virtual environment can process its .pth files more than once.
    if path in _parents:
        return
    _parents.append(path)
    if _original_execsitecustomize is None:
        _original_addpackage = site.addpackage
        _original_execsitecustomize = site.execsitecustomize
        site.addpackage = _addpackage
        site.execsitecustomize = _finish
    before = {id(entry) for entry in sys.path}
    _adding_parent += 1
    try:
        # Later executable .pth files may import shared dependencies.
        site.addsitedir(path)
    finally:
        _parent_paths.extend(entry for entry in sys.path if id(entry) not in before)
        _adding_parent -= 1


def _move_parent_paths():
    parent_ids = {id(entry) for entry in _parent_paths}
    # Move only entries introduced by the parents, leaving existing paths alone.
    sys.path[:] = [entry for entry in sys.path if id(entry) not in parent_ids] + [
        entry for entry in sys.path if id(entry) in parent_ids
    ]


def _addpackage(*args, **kwargs):
    if not _adding_parent:
        _move_parent_paths()
    try:
        return _original_addpackage(*args, **kwargs)
    finally:
        if not _adding_parent:
            # An editable path must precede shared packages before the next hook.
            _move_parent_paths()


def _finish():
    global _finished, _original_addpackage, _original_execsitecustomize
    original = _original_execsitecustomize
    try:
        _move_parent_paths()
    finally:
        _finished = True
        site.addpackage = _original_addpackage
        site.execsitecustomize = original
        _original_addpackage = None
        _original_execsitecustomize = None
        _parents.clear()
        _parent_paths.clear()
    original()
