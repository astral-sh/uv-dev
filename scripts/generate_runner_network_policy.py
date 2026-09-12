"""Compile the runner's reviewed policies for use before checkout."""

import argparse
import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ACTION = ROOT / ".github/actions/runner-network-policy"
sys.path.insert(0, str(ACTION))

from policy import Policy


def generate(root=ROOT):
    source = tomllib.loads((root / ".github/runner-network-policy.toml").read_text())
    codex = tomllib.loads((root / "agents/codex/config.toml").read_text())
    profiles = {}
    for name, profile in source["profiles"].items():
        allow = set()
        deny = set()
        for group in profile.get("groups", []):
            allow.update(source["groups"][group].get("allow", []))
            deny.update(source["groups"][group].get("deny", []))
        for imported in profile.get("codex", []):
            permission = codex["permissions"][imported]
            if permission.get("extends") not in {":workspace", ":read-only"}:
                raise ValueError("unsupported permission inheritance")
            network = permission["network"]
            if network.get("enabled") is not True:
                raise ValueError("imported network policy is disabled")
            for domain, rule in network["domains"].items():
                if rule == "allow":
                    allow.add(domain)
                elif rule == "deny":
                    deny.add(domain)
                else:
                    raise ValueError("unsupported domain rule")
        value = {"allow": sorted(allow), "deny": sorted(deny)}
        Policy.from_dict(value)
        profiles[name] = value
    for job in source["jobs"].values():
        if job["profile"] not in profiles or job["privileges"] not in {
            "drop",
            "retain",
        }:
            raise ValueError("invalid job policy")
    return (
        json.dumps(
            {
                "version": 1,
                "source_revision": source["source_revision"],
                "profiles": profiles,
                "jobs": source["jobs"],
            },
            indent=2,
            sort_keys=True,
        )
        + "\n"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = ACTION / "policies.json"
    expected = generate()
    if args.check:
        if path.read_text() != expected:
            raise SystemExit("Run scripts/generate_runner_network_policy.py")
    else:
        path.write_text(expected)


if __name__ == "__main__":
    main()
