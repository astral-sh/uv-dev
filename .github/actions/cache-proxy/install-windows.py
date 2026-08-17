"""Windows default-deny prototype; direct-IP firewall isolation is not provided."""

import ipaddress
import json
import os
import shutil
import signal
import subprocess
import sys
from pathlib import Path

DIRECTORY = Path(os.environ["ProgramData"]) / "uv-cache-proxy"
HOSTS = Path(os.environ["SystemRoot"]) / "System32/drivers/etc/hosts"
MARKER = "# uv-cache-proxy-probe"


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
    if DIRECTORY.exists():
        raise RuntimeError("prototype already installed")
    DIRECTORY.mkdir()
    run(
        "icacls",
        str(DIRECTORY),
        "/inheritance:r",
        "/grant:r",
        "*S-1-5-18:(OI)(CI)F",
        "*S-1-5-32-544:(OI)(CI)F",
    )
    shutil.copyfile(Path(__file__).with_name("proxy.py"), DIRECTORY / "proxy.py")
    (DIRECTORY / "origins.json").write_text(json.dumps(origins) + "\n")
    (DIRECTORY / "audit.json").write_text("{}\n")
    (DIRECTORY / "ca.cnf").write_text(
        "[req]\ndistinguished_name=dn\nx509_extensions=ca\nprompt=no\n[dn]\nCN=uv disposable cache proxy\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n"
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
        "/CN=uv disposable cache proxy",
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
    run("certutil", "-addstore", "Root", str(DIRECTORY / "ca.crt"))
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
    with (DIRECTORY / "service-error.log").open("wb") as errors:
        process = subprocess.Popen(
            [sys.executable, str(DIRECTORY / "proxy.py"), str(DIRECTORY)],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=errors,
            creationflags=subprocess.DETACHED_PROCESS
            | subprocess.CREATE_NEW_PROCESS_GROUP,
        )
    (DIRECTORY / "pid").write_text(str(process.pid))
    with HOSTS.open("a") as hosts:
        hosts.write("\n127.0.0.1 " + " ".join(origins) + " " + MARKER + "\n")
    run("ipconfig", "/flushdns")


def cleanup():
    if not DIRECTORY.exists():
        return
    HOSTS.write_text(
        "".join(
            line
            for line in HOSTS.read_text().splitlines(keepends=True)
            if MARKER not in line
        )
    )
    run("ipconfig", "/flushdns")
    pid = DIRECTORY / "pid"
    if pid.exists():
        try:
            os.kill(int(pid.read_text()), signal.SIGTERM)
        except ProcessLookupError:
            pass
    fingerprint = DIRECTORY / "ca-fingerprint"
    if fingerprint.exists():
        run("certutil", "-delstore", "Root", fingerprint.read_text().strip())


if __name__ == "__main__":
    if sys.argv[1] == "install":
        install(sys.argv[2])
    elif sys.argv[1] == "cleanup":
        cleanup()
    else:
        raise SystemExit("unknown operation")
