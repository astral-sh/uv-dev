"""Inspect uv's requirement parsing and marker evaluation."""

import argparse
import json
from pathlib import Path

from . import Requirement


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("requirement")
    parser.add_argument(
        "--environment",
        type=Path,
        help="JSON file containing a complete PEP 508 environment",
    )
    parser.add_argument("--extra", action="append", default=[], help="An active extra")
    args = parser.parse_args()
    if args.extra and args.environment is None:
        parser.error("--extra requires --environment")

    try:
        requirement = Requirement(args.requirement)
        result: dict[str, object] = {
            "name": requirement.name,
            "extras": requirement.extras,
            "specifiers": str(requirement.specifiers)
            if requirement.specifiers
            else None,
            "url": requirement.url,
            "marker": requirement.marker,
        }
        if args.environment is not None:
            environment = json.loads(args.environment.read_text(encoding="utf-8"))
            result["applies"] = requirement.evaluate(environment, extras=args.extra)
    except (OSError, TypeError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
