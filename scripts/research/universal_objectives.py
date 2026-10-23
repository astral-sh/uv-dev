"""Compare uv universal choices with a finite, exact multi-objective frontier."""

import argparse
import base64
import hashlib
import itertools
import json
import os
import re
import subprocess
import tempfile
import zipfile
from pathlib import Path

PACKAGES = ("a", "b", "common")
PYTHON_MINORS = (11, 12)
# All roots and dependencies are mandatory in both environment cells.
RELEASES = {
    "a": {1: (11, ("common>=1,<3",)), 2: (12, ("common==2",))},
    "b": {1: (11, ("common==1",)), 2: (12, ("common==2",))},
    "common": {1: (11, ()), 2: (12, ())},
}


def write_wheels(directory: Path) -> str:
    fingerprint = hashlib.sha256()
    for name, releases in RELEASES.items():
        for version, (minimum, dependencies) in releases.items():
            info = f"{name}-{version}.dist-info"
            metadata = f"Metadata-Version: 2.3\nName: {name}\nVersion: {version}\nRequires-Python: >=3.{minimum}\n"
            metadata += "".join(
                f"Requires-Dist: {dependency}\n" for dependency in dependencies
            )
            files = {
                f"{info}/METADATA": metadata.encode(),
                f"{info}/WHEEL": b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
            }
            record = ""
            for path, data in files.items():
                digest = (
                    base64.urlsafe_b64encode(hashlib.sha256(data).digest())
                    .rstrip(b"=")
                    .decode()
                )
                record += f"{path},sha256={digest},{len(data)}\n"
            files[f"{info}/RECORD"] = f"{record}{info}/RECORD,,\n".encode()
            path = directory / f"{name}-{version}-py3-none-any.whl"
            with zipfile.ZipFile(path, "w") as archive:
                for filename, data in files.items():
                    archive.writestr(
                        zipfile.ZipInfo(filename, (2020, 1, 1, 0, 0, 0)), data
                    )
            fingerprint.update(path.name.encode())
            fingerprint.update(path.read_bytes())
    return fingerprint.hexdigest()


def valid(assignment: tuple[int, ...], minor: int) -> bool:
    a, b, common = assignment
    # These predicates independently encode this fixture's complete dependency rules.
    return (
        all(
            RELEASES[name][version][0] <= minor
            for name, version in zip(PACKAGES, assignment, strict=True)
        )
        and (a == 1 or common == 2)
        and common == b
    )


def objectives(solution: tuple[tuple[int, ...], ...]) -> tuple[int, int, int]:
    distinct = sum(
        len({assignment[index] for assignment in solution})
        for index in range(len(PACKAGES))
    )
    freshness = sum(
        max(
            version
            for version, (minimum, _) in RELEASES[name].items()
            if minimum <= minor
        )
        - selected
        for minor, assignment in zip(PYTHON_MINORS, solution, strict=True)
        for name, selected in zip(PACKAGES, assignment, strict=True)
    )
    # The reference assignment selects version 1 everywhere.
    churn = sum(selected != 1 for assignment in solution for selected in assignment)
    return distinct, freshness, churn


def dominates(left: tuple[int, ...], right: tuple[int, ...]) -> bool:
    return all(a <= b for a, b in zip(left, right, strict=True)) and left != right


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    args = parser.parse_args()
    uv = args.uv.resolve(strict=True)
    with uv.open("rb") as executable:
        binary_sha256 = hashlib.file_digest(executable, "sha256").hexdigest()
    args.work_dir = args.work_dir.resolve()
    args.work_dir.mkdir(parents=True, exist_ok=True)
    environment = {
        key: value for key, value in os.environ.items() if not key.startswith("UV_")
    }
    environment.update(UV_NO_CONFIG="1", UV_PYTHON_DOWNLOADS="never", NO_COLOR="1")
    candidates = [
        [
            assignment
            for assignment in itertools.product((1, 2), repeat=3)
            if valid(assignment, minor)
        ]
        for minor in PYTHON_MINORS
    ]
    feasible = list(itertools.product(*candidates))
    vectors = sorted({objectives(solution) for solution in feasible})
    frontier = [
        vector
        for vector in vectors
        if not any(dominates(other, vector) for other in vectors)
    ]
    minima = tuple(min(vector[index] for vector in vectors) for index in range(3))
    measurements = {}
    preference_measurements = {}
    with tempfile.TemporaryDirectory(
        prefix="universal-objectives-", dir=args.work_dir
    ) as temporary:
        directory = Path(temporary)
        environment["TMPDIR"] = str(directory)
        wheelhouse = directory / "wheels"
        wheelhouse.mkdir()
        fingerprint = write_wheels(wheelhouse)
        project = directory / "pyproject.toml"
        project.write_text(
            '[project]\nname = "objective-fixture"\nversion = "1"\nrequires-python = ">=3.11,<3.13"\ndependencies = ["a", "b"]\n'
        )
        common_args = [
            "--offline",
            "--no-index",
            "--find-links",
            str(wheelhouse),
            "--no-build",
            "--no-cache",
            "--no-header",
            "--no-annotate",
            "--color",
            "never",
        ]

        def compile_requirements(
            source: Path, options: list[str], destination: Path | None = None
        ) -> str:
            if destination is not None:
                options = [*options, "--output-file", str(destination)]
            result = subprocess.run(
                [str(uv), "pip", "compile", str(source), *common_args, *options],
                cwd=directory,
                env=environment,
                capture_output=True,
                text=True,
                timeout=60,
                check=False,
            )
            if result.returncode:
                raise RuntimeError(result.stderr)
            return result.stdout if destination is None else destination.read_text()

        def project_output(label: str, output: str) -> tuple[tuple[int, ...], ...]:
            universal = directory / f"{label}.txt"
            universal.write_text(output)
            projected = []
            for minor in PYTHON_MINORS:
                concrete = compile_requirements(
                    universal,
                    [
                        "--python-version",
                        f"3.{minor}",
                        "--python-platform",
                        "linux",
                        "--no-deps",
                    ],
                )
                pins = {}
                for line in concrete.splitlines():
                    if not line.strip() or line.startswith("#"):
                        continue
                    match = re.fullmatch(r"(a|b|common)==([12])", line)
                    if match is None or match[1] in pins:
                        raise RuntimeError(f"unsupported concrete output: {line!r}")
                    pins[match[1]] = int(match[2])
                if set(pins) != set(PACKAGES):
                    raise RuntimeError(f"incomplete projection: {pins}")
                assignment = tuple(pins[name] for name in PACKAGES)
                if not valid(assignment, minor):
                    raise RuntimeError(
                        f"uv projection violates the finite oracle: {assignment}"
                    )
                projected.append(assignment)
            return tuple(projected)

        universal_options = [
            "--universal",
            "--python-version",
            "3.11",
            "--no-strip-markers",
        ]
        reference_output = compile_requirements(
            project, [*universal_options, "--resolution", "lowest"]
        )
        reference_solution = project_output("reference", reference_output)
        if reference_solution != ((1, 1, 1), (1, 1, 1)):
            raise RuntimeError(
                f"unexpected lowest-resolution reference: {reference_solution}"
            )
        projection_controls = {}
        for label, output, expected_error in [
            (
                "missing-transitive-pin",
                "\n".join(
                    line
                    for line in reference_output.splitlines()
                    if not line.startswith("common==")
                ),
                "incomplete projection",
            ),
            (
                "inconsistent-transitive-pin",
                reference_output.replace(
                    "common==1",
                    "common==1 ; python_full_version < '3.12'\ncommon==2 ; python_full_version >= '3.12'",
                ),
                "violates the finite oracle",
            ),
        ]:
            try:
                project_output(label, output)
            except RuntimeError as error:
                if expected_error not in str(error):
                    raise
                projection_controls[label] = "rejected"
            else:
                raise RuntimeError(
                    f"projection control {label} was unexpectedly accepted"
                )
        for strategy in ("requires-python", "fewest"):
            preference_measurements[strategy] = {}
            for mode in ("cold", "prefer-reference", "upgrade-reference"):
                options = [*universal_options, "--fork-strategy", strategy]
                destination = None
                if mode != "cold":
                    destination = directory / f"{strategy}-{mode}-previous.txt"
                    destination.write_text(reference_output)
                if mode == "upgrade-reference":
                    options.append("--upgrade")
                output = compile_requirements(project, options, destination)
                solution = project_output(f"{strategy}-{mode}", output)
                vector = objectives(solution)
                measurement = {
                    "requirements": output,
                    "projected_assignments": solution,
                    "objectives": vector,
                    "pareto_optimal": vector in frontier,
                    "independent_objective_gaps": tuple(
                        value - minimum
                        for value, minimum in zip(vector, minima, strict=True)
                    ),
                    "previous_output_supplied": destination is not None,
                    "upgrade": mode == "upgrade-reference",
                }
                if mode == "cold":
                    measurements[strategy] = measurement
                else:
                    preference_measurements[strategy][mode] = measurement
        version = subprocess.check_output(
            [str(uv), "--version"], text=True, env=environment
        ).strip()
        print(
            json.dumps(
                {
                    "uv": str(uv),
                    "binary_sha256": binary_sha256,
                    "version": version,
                    "fixture_sha256": fingerprint,
                    "packages": PACKAGES,
                    "python_minors": PYTHON_MINORS,
                    "objective_order": [
                        "distinct_versions",
                        "freshness_rank_gap",
                        "changed_environment_pins",
                    ],
                    "churn_reference": {
                        "assignments": [[1, 1, 1], [1, 1, 1]],
                        "source": "uv pip compile --universal --resolution lowest",
                        "requirements": reference_output,
                        "provided_to_uv_in": ["prefer-reference", "upgrade-reference"],
                    },
                    "feasible_solution_count": len(feasible),
                    "pareto_frontier": frontier,
                    "independent_minima": minima,
                    "measurements": measurements,
                    "preference_measurements": preference_measurements,
                    "projection_controls": projection_controls,
                },
                indent=2,
            )
        )


if __name__ == "__main__":
    main()
