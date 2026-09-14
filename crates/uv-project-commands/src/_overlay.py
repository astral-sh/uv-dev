"""Add the site directories belonging to a uv run environment."""

import site

SITE_PACKAGES = ()
_applied = False


def apply():
    """Process the overlay's site directories once, in precedence order."""
    global _applied
    if _applied:
        return
    _applied = True

    if hasattr(site, "StartupState"):
        state = site.StartupState()
        for path in SITE_PACKAGES:
            state.addsitedir(path)
        state.process()
    else:
        for path in SITE_PACKAGES:
            site.addsitedir(path)
