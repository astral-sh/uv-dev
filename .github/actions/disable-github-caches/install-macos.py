"""macOS port of the disposable proxy; use only in an ephemeral test job."""

import ipaddress
import json
import os
import plistlib
import pwd
import re
import shutil
import subprocess
import sys
from pathlib import Path

DIRECTORY = Path("/var/run/uv-cache-proxy")
USER = "_uvcacheproxy"
LABEL = "sh.astral.uv-cache-proxy"
PLIST = Path("/Library/LaunchDaemons") / f"{LABEL}.plist"
ANCHOR = "com.apple/uv-cache-proxy"
MARKER = "# uv-release-cache-proxy"


def run(*args, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True)


def install(plan):
    origins = json.loads(Path(plan).read_text())
    for hostname, origin in origins.items():
        if not hostname.endswith(".actions.githubusercontent.com") or any(
            character not in "abcdefghijklmnopqrstuvwxyz0123456789.-"
            for character in hostname
        ):
            raise ValueError("unexpected hostname")
        for address in origin["addresses"]:
            if not ipaddress.ip_address(address).is_global:
                raise ValueError("unexpected upstream address")
    if DIRECTORY.exists() or PLIST.exists():
        raise RuntimeError("cache proxy already installed")
    used = {account.pw_uid for account in pwd.getpwall()}
    uid = next(value for value in range(400, 500) if value not in used)
    run("dscl", ".", "-create", f"/Users/{USER}")
    for key, value in {
        "UniqueID": str(uid),
        "PrimaryGroupID": "1",
        "NFSHomeDirectory": "/var/empty",
        "UserShell": "/usr/bin/false",
        "IsHidden": "1",
    }.items():
        run("dscl", ".", "-create", f"/Users/{USER}", key, value)
    DIRECTORY.mkdir(mode=0o755)
    shutil.copyfile(Path(__file__).with_name("proxy.py"), DIRECTORY / "proxy.py")
    (DIRECTORY / "origins.json").write_text(json.dumps(origins) + "\n")
    (DIRECTORY / "audit.json").write_text("{}\n")
    (DIRECTORY / "ca.cnf").write_text(
        "[req]\ndistinguished_name=dn\nx509_extensions=ca\nprompt=no\n[dn]\nCN=uv release cache proxy\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n"
    )
    run(
        "openssl",
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-days",
        "1",
        "-config",
        str(DIRECTORY / "ca.cnf"),
        "-keyout",
        str(DIRECTORY / "ca.key"),
        "-out",
        str(DIRECTORY / "ca.crt"),
    )
    run(
        "openssl",
        "req",
        "-new",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-subj",
        "/CN=uv release cache proxy",
        "-keyout",
        str(DIRECTORY / "server.key"),
        "-out",
        str(DIRECTORY / "server.csr"),
    )
    (DIRECTORY / "extensions").write_text(
        "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName="
        + ",".join(f"DNS:{hostname}" for hostname in origins)
        + "\n"
    )
    run(
        "openssl",
        "x509",
        "-req",
        "-in",
        str(DIRECTORY / "server.csr"),
        "-CA",
        str(DIRECTORY / "ca.crt"),
        "-CAkey",
        str(DIRECTORY / "ca.key"),
        "-CAcreateserial",
        "-days",
        "1",
        "-extfile",
        str(DIRECTORY / "extensions"),
        "-out",
        str(DIRECTORY / "server.crt"),
    )
    (DIRECTORY / "ca.key").unlink()
    os.chown(DIRECTORY, uid, 1)
    for name in ("server.key", "audit.json"):
        os.chown(DIRECTORY / name, uid, 1)
    (DIRECTORY / "server.key").chmod(0o600)
    run(
        "security",
        "add-trusted-cert",
        "-d",
        "-r",
        "trustRoot",
        "-k",
        "/Library/Keychains/System.keychain",
        str(DIRECTORY / "ca.crt"),
    )
    fingerprint = (
        run(
            "openssl",
            "x509",
            "-in",
            str(DIRECTORY / "ca.crt"),
            "-noout",
            "-fingerprint",
            "-sha1",
        )
        .stdout.strip()
        .split("=", 1)[1]
        .replace(":", "")
    )
    (DIRECTORY / "ca-fingerprint").write_text(fingerprint)
    PLIST.write_bytes(
        plistlib.dumps(
            {
                "Label": LABEL,
                "ProgramArguments": [
                    "/usr/bin/python3",
                    str(DIRECTORY / "proxy.py"),
                    str(DIRECTORY),
                    "--uid",
                    str(uid),
                    "--gid",
                    "1",
                ],
                "RunAtLoad": True,
                "KeepAlive": True,
                "StandardErrorPath": str(DIRECTORY / "service-error.log"),
            }
        )
    )
    PLIST.chmod(0o644)
    run("launchctl", "bootstrap", "system", str(PLIST))
    with Path("/etc/hosts").open("a") as hosts:
        hosts.write("\n127.0.0.1 " + " ".join(origins) + " " + MARKER + "\n")
    run("dscacheutil", "-flushcache")
    run("killall", "-HUP", "mDNSResponder", check=False)
    addresses = sorted(
        {address for origin in origins.values() for address in origin["addresses"]}
    )
    targets = "{ " + ", ".join(addresses) + " }"
    (DIRECTORY / "pf.rules").write_text(
        f"pass out quick proto tcp to {targets} port 443 user {uid} keep state\nblock return out quick proto tcp to {targets} port 443\n"
    )
    run("pfctl", "-a", ANCHOR, "-f", str(DIRECTORY / "pf.rules"))
    enabled = run("pfctl", "-E")
    match = re.search(r"Token\s*:\s*(\d+)", enabled.stdout + enabled.stderr)
    if not match:
        raise RuntimeError("PF enable token missing")
    (DIRECTORY / "pf-token").write_text(match.group(1))
    for address in addresses:
        run(
            "pfctl",
            "-k",
            "::/0" if ":" in address else "0.0.0.0/0",
            "-k",
            address,
            check=False,
        )


def cleanup():
    if not DIRECTORY.exists():
        return
    run("pfctl", "-a", ANCHOR, "-F", "rules", check=False)
    token = DIRECTORY / "pf-token"
    if token.exists():
        run("pfctl", "-X", token.read_text().strip(), check=False)
    hosts = Path("/etc/hosts")
    hosts.write_text(
        "".join(
            line
            for line in hosts.read_text().splitlines(keepends=True)
            if MARKER not in line
        )
    )
    run("dscacheutil", "-flushcache")
    run("killall", "-HUP", "mDNSResponder", check=False)
    run("launchctl", "bootout", "system", str(PLIST), check=False)
    fingerprint = DIRECTORY / "ca-fingerprint"
    if fingerprint.exists():
        run(
            "security",
            "delete-certificate",
            "-Z",
            fingerprint.read_text().strip(),
            "/Library/Keychains/System.keychain",
        )
    PLIST.unlink(missing_ok=True)
    run("dscl", ".", "-delete", f"/Users/{USER}", check=False)


if __name__ == "__main__":
    if os.geteuid() != 0:
        raise SystemExit("root is required")
    if sys.argv[1] == "install":
        install(sys.argv[2])
    elif sys.argv[1] == "cleanup":
        cleanup()
    else:
        raise SystemExit("unknown operation")
