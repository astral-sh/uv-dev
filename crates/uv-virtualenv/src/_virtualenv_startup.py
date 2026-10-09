"""Install the virtual environment's startup hook."""

import _virtualenv


def patch():
    """Install a callable hook unless a legacy import-time hook is already active."""
    install_patch = getattr(_virtualenv, "patch", None)
    if install_patch is not None:
        install_patch()
