"""Install/remove the prototype only on its disposable hosted Linux runner."""

import ipaddress
import json
import os
import pwd
import shutil
import subprocess
import sys
from pathlib import Path

DIRECTORY = Path("/run/uv-cache-proxy")
CA = Path("/usr/local/share/ca-certificates/uv-cache-proxy.crt")
USER = "uv-cache-proxy"
CHAIN = "UV_CACHE_PROXY"
NAT_CHAIN = "UV_CACHE_PROXY_NAT"
MARKER = "# uv-cache-proxy-probe"


def run(*args, check=True):
    return subprocess.run(
        args, check=check, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )


def validate(origins):
    if not 1 <= len(origins) <= 8:
        raise ValueError("unexpected origin count")
    for hostname, value in origins.items():
        if value.get("scheme") == "http":
            address = ipaddress.ip_address(value["addresses"][0])
            if (
                not address.is_private
                or address.version != 4
                or value["port"] not in (977, 978)
                or value["listen_port"] != value["port"] + 19000
                or hostname != f"{address}:{value['port']}"
            ):
                raise ValueError("unexpected private cache endpoint")
            continue
        if not hostname.endswith(".actions.githubusercontent.com") or any(
            character not in "abcdefghijklmnopqrstuvwxyz0123456789.-"
            for character in hostname
        ):
            raise ValueError("unexpected service hostname")
        if not value["addresses"]:
            raise ValueError("missing service addresses")
        for address in value["addresses"]:
            parsed = ipaddress.ip_address(address)
            if not parsed.is_global:
                raise ValueError("non-public upstream address")


def install(plan):
    origins = json.loads(Path(plan).read_text())
    validate(origins)
    secure_origins = {
        name: value for name, value in origins.items() if value.get("scheme") != "http"
    }
    if DIRECTORY.exists() or CA.exists():
        raise RuntimeError("prototype already installed")
    run("useradd", "--system", "--no-create-home", "--shell", "/usr/sbin/nologin", USER)
    account = pwd.getpwnam(USER)
    DIRECTORY.mkdir(mode=0o755)
    shutil.copyfile(Path(__file__).with_name("proxy.py"), DIRECTORY / "proxy.py")
    (DIRECTORY / "origins.json").write_text(json.dumps(origins) + "\n")
    (DIRECTORY / "audit.json").write_text("{}\n")
    run(
        "openssl",
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-days",
        "1",
        "-subj",
        "/CN=uv disposable cache proxy",
        "-keyout",
        str(DIRECTORY / "ca.key"),
        "-out",
        str(DIRECTORY / "ca.crt"),
        "-addext",
        "basicConstraints=critical,CA:TRUE",
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
        + ",".join(f"DNS:{hostname}" for hostname in secure_origins)
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
    # The signing key is never needed by the running proxy.
    (DIRECTORY / "ca.key").unlink()
    os.chown(DIRECTORY, account.pw_uid, account.pw_gid)
    for name in ("server.key", "audit.json"):
        os.chown(DIRECTORY / name, account.pw_uid, account.pw_gid)
    (DIRECTORY / "server.key").chmod(0o600)
    shutil.copyfile(DIRECTORY / "ca.crt", CA)
    run("update-ca-certificates")
    run(
        "systemd-run",
        "--quiet",
        "--collect",
        "--unit=uv-cache-proxy",
        "--service-type=exec",
        f"--property=User={USER}",
        "--property=AmbientCapabilities=CAP_NET_BIND_SERVICE",
        "--property=NoNewPrivileges=yes",
        "/usr/bin/python3",
        str(DIRECTORY / "proxy.py"),
        str(DIRECTORY),
    )
    # Steering the exact service names also catches clients that ignore proxy env.
    with Path("/etc/hosts").open("a") as hosts:
        hosts.write("\n127.0.0.1 " + " ".join(secure_origins) + " " + MARKER + "\n")
    addresses = {
        address for origin in secure_origins.values() for address in origin["addresses"]
    }
    for program, version in (("iptables", 4), ("ip6tables", 6)):
        selected = sorted(
            address
            for address in addresses
            if ipaddress.ip_address(address).version == version
        )
        if not selected:
            continue
        run(program, "-N", CHAIN)
        run(
            program,
            "-A",
            CHAIN,
            "-m",
            "owner",
            "--uid-owner",
            str(account.pw_uid),
            "-j",
            "RETURN",
        )
        for address in selected:
            run(
                program,
                "-A",
                CHAIN,
                "-d",
                address,
                "-p",
                "tcp",
                "--dport",
                "443",
                "-j",
                "REJECT",
                "--reject-with",
                "tcp-reset",
            )
        run(program, "-I", "OUTPUT", "1", "-j", CHAIN)
    private_origins = [
        origin for origin in origins.values() if origin.get("scheme") == "http"
    ]
    if private_origins:
        run("iptables", "-t", "nat", "-N", NAT_CHAIN)
        run(
            "iptables",
            "-t",
            "nat",
            "-A",
            NAT_CHAIN,
            "-m",
            "owner",
            "--uid-owner",
            str(account.pw_uid),
            "-j",
            "RETURN",
        )
        for origin in private_origins:
            run(
                "iptables",
                "-t",
                "nat",
                "-A",
                NAT_CHAIN,
                "-d",
                origin["addresses"][0],
                "-p",
                "tcp",
                "--dport",
                str(origin["port"]),
                "-j",
                "REDIRECT",
                "--to-ports",
                str(origin["listen_port"]),
            )
        run("iptables", "-t", "nat", "-I", "OUTPUT", "1", "-j", NAT_CHAIN)


def cleanup():
    if not DIRECTORY.exists():
        return
    if run("iptables", "-t", "nat", "-S", NAT_CHAIN, check=False).returncode == 0:
        run("iptables", "-t", "nat", "-D", "OUTPUT", "-j", NAT_CHAIN, check=False)
        run("iptables", "-t", "nat", "-F", NAT_CHAIN)
        run("iptables", "-t", "nat", "-X", NAT_CHAIN)
    for program in ("iptables", "ip6tables"):
        if run(program, "-S", CHAIN, check=False).returncode == 0:
            run(program, "-D", "OUTPUT", "-j", CHAIN, check=False)
            run(program, "-F", CHAIN)
            run(program, "-X", CHAIN)
    hosts = Path("/etc/hosts")
    hosts.write_text(
        "".join(
            line
            for line in hosts.read_text().splitlines(keepends=True)
            if MARKER not in line
        )
    )
    run("systemctl", "stop", "uv-cache-proxy.service", check=False)
    CA.unlink(missing_ok=True)
    run("update-ca-certificates")


if __name__ == "__main__":
    if os.geteuid() != 0:
        raise SystemExit("root is required")
    if sys.argv[1] == "install":
        install(sys.argv[2])
    elif sys.argv[1] == "cleanup":
        cleanup()
    else:
        raise SystemExit("unknown operation")
