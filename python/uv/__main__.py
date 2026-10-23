import os
import sys

from uv import find_uv_bin


def _detect_virtualenv() -> str:
    """Return the virtual environment path, or an empty string if none exists."""

    # Use `VIRTUAL_ENV` when it is already set.
    value = os.getenv("VIRTUAL_ENV")
    if value:
        return value

    # Check whether the current Python prefix contains `pyvenv.cfg`.
    venv_marker = os.path.join(sys.prefix, "pyvenv.cfg")

    if os.path.exists(venv_marker):
        return sys.prefix

    return ""


def _run() -> None:
    """Run `uv` with the current Python interpreter and virtual environment."""
    uv = find_uv_bin()

    env = os.environ.copy()
    venv = _detect_virtualenv()
    if venv:
        env.setdefault("VIRTUAL_ENV", venv)

    # Tell `uv` which Python interpreter started it.
    env["UV_INTERNAL__PARENT_INTERPRETER"] = sys.executable

    if sys.platform == "win32":
        import subprocess

        # Exit without a traceback when the user interrupts the process.
        try:
            completed_process = subprocess.run(
                [uv, *sys.argv[1:]], env=env, check=False
            )
        except KeyboardInterrupt:
            sys.exit(2)

        sys.exit(completed_process.returncode)
    else:
        os.execvpe(uv, [uv, *sys.argv[1:]], env=env)


if __name__ == "__main__":
    _run()
