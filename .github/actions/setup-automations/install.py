"""Install the checked-in automation package without an editable source link."""

import os
import subprocess
import sys
from pathlib import Path
from tempfile import mkdtemp


def main() -> None:
    if sys.version_info < (3, 14):
        raise RuntimeError("The automation runtime requires Python 3.14 or newer")

    project = Path(__file__).resolve().parents[3] / "scripts" / "automations"
    directory = Path(mkdtemp(prefix="uv-automations-", dir=os.environ["RUNNER_TEMP"]))
    environment = os.environ.copy()
    environment.pop("VIRTUAL_ENV", None)
    environment["UV_PROJECT_ENVIRONMENT"] = str(directory)
    subprocess.run(
        [
            "uv",
            "sync",
            "--directory",
            str(project),
            "--project",
            str(project),
            "--locked",
            "--no-dev",
            "--no-editable",
            # The source may have changed at the same checkout path. Always
            # rebuild this tiny package, and keep the runtime independent of
            # writable wheel-cache files.
            "--reinstall-package",
            "uv-automations",
            "--link-mode",
            "copy",
            "--python",
            sys.executable,
        ],
        env=environment,
        check=True,
    )

    scripts = directory / ("Scripts" if os.name == "nt" else "bin")
    suffix = ".exe" if os.name == "nt" else ""
    outputs = {
        "executable": scripts / f"uv-automations{suffix}",
        "python": scripts / f"python{suffix}",
    }
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        for name, path in outputs.items():
            if not path.is_file() or any(
                character in str(path) for character in "\r\n"
            ):
                raise RuntimeError(f"The installed {name} path is invalid")
            output.write(f"{name}={path}\n")


if __name__ == "__main__":
    main()
