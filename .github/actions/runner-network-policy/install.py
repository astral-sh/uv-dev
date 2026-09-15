"""Root-only setup for a disposable Linux runner. Never run on a workstation."""

import argparse
import ipaddress
import json
import os
import pwd
import re
import shutil
import socket
import stat
import subprocess
import time
from pathlib import Path

from policy import validate_private_origins

DIRECTORY = Path("/run/uv-network-policy")
USER = "uv-network-policy"
TABLE = "uv_network_policy"
SERVICE = "uv-network-policy.service"


def run(*arguments, capture=False, check=True):
    return subprocess.run(
        arguments,
        check=check,
        text=True,
        stdout=subprocess.PIPE if capture else subprocess.DEVNULL,
        stderr=subprocess.PIPE if capture else None,
    )


def resolvers():
    result = []
    for path in (Path("/run/systemd/resolve/resolv.conf"), Path("/etc/resolv.conf")):
        if not path.exists():
            continue
        for line in path.read_text().splitlines():
            fields = line.split()
            if len(fields) < 2 or fields[0] != "nameserver":
                continue
            address = ipaddress.ip_address(fields[1])
            if not address.is_loopback and not address.is_unspecified:
                result.append(str(address))
    if not result:
        raise RuntimeError("no upstream DNS resolver found")
    return list(dict.fromkeys(result))


def proc_address(value, version):
    address, port = value.split(":")
    packed = bytes.fromhex(address)
    packed = b"".join(
        packed[index : index + 4][::-1] for index in range(0, len(packed), 4)
    )
    family = socket.AF_INET if version == 4 else socket.AF_INET6
    return socket.inet_ntop(family, packed), int(port, 16)


def bootstrap_connections(runner_uid):
    """Retain only existing runner-owned HTTPS flows, not arbitrary conntrack."""
    result = []
    for version, name in ((4, "tcp"), (6, "tcp6")):
        for line in Path(f"/proc/net/{name}").read_text().splitlines()[1:]:
            fields = line.split()
            if fields[3] != "01" or int(fields[7]) != runner_uid:
                continue
            _, source_port = proc_address(fields[1], version)
            address, port = proc_address(fields[2], version)
            if port == 443 and ipaddress.ip_address(address).is_global:
                result.append((version, address, source_port, port))
    return sorted(set(result))


def firewall(proxy_uid, runner_uid, dns_servers, connections, private_services=None):
    rules = []
    for resolver in dns_servers:
        address = ipaddress.ip_address(resolver)
        family = "ip" if address.version == 4 else "ip6"
        rules.append(
            f"meta skuid {proxy_uid} {family} daddr {address} udp dport 53 accept"
        )
        rules.append(
            f"meta skuid {proxy_uid} {family} daddr {address} tcp dport 53 accept"
        )
    for version, address, source_port, port in connections:
        family = "ip" if version == 4 else "ip6"
        rules.append(
            f"meta skuid {runner_uid} ct state established {family} daddr {ipaddress.ip_address(address)} tcp sport {int(source_port)} tcp dport {int(port)} accept"
        )
    exceptions = "\n    ".join(rules)
    redirects = "\n    ".join(
        f"ip daddr {authority.split(':')[0]} tcp dport {authority.split(':')[1]} redirect to :18080"
        for authority in validate_private_origins(private_services or {})
    )
    return f"""table inet {TABLE} {{
  chain steer {{
    type nat hook output priority dstnat; policy accept;
    meta skuid {proxy_uid} return
    {redirects}
    udp dport 53 redirect to :1053
    tcp dport 53 redirect to :1053
    oifname \"lo\" return
    tcp dport 80 redirect to :18080
    tcp dport 443 redirect to :18443
  }}
  chain egress {{
    type filter hook output priority -10; policy accept;
    oifname \"lo\" accept
    ip daddr 127.0.0.1 tcp dport {{ 1053, 18080, 18443 }} accept
    ip daddr 127.0.0.1 udp dport 1053 accept
    ip6 daddr ::1 tcp dport {{ 1053, 18080, 18443 }} accept
    ip6 daddr ::1 udp dport 1053 accept
    {exceptions}
    meta skuid {proxy_uid} tcp dport {{ 80, 443 }} accept
    reject with icmpx type admin-prohibited
  }}
  chain forwarded {{
    type filter hook forward priority -10; policy accept;
    reject with icmpx type admin-prohibited
  }}
  chain ingress {{
    type filter hook input priority -10; policy accept;
    iifname != \"lo\" tcp dport {{ 1053, 18080, 18443 }} reject
    iifname != \"lo\" udp dport 1053 reject
  }}
}}
"""


def health():
    with socket.create_connection(("127.0.0.1", 18080), timeout=2) as stream:
        stream.sendall(
            b"GET /__uv_network_proxy_health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        if b" 204 " not in stream.recv(1024).split(b"\r\n", 1)[0]:
            raise RuntimeError("proxy health check failed")


def normalize_runner_sudoers(path):
    if path.exists():
        metadata = path.lstat()
        if metadata.st_uid != 0 or not stat.S_ISREG(metadata.st_mode):
            raise RuntimeError("unexpected runner sudoers file")
        path.chmod(0o440)


def drop_privileges(account):
    # Existing supplementary groups are retained by Runner.Worker. Merely
    # removing the user from the docker group does not revoke socket access.
    for unit in ("docker.socket", "docker.service", "containerd.service"):
        run("systemctl", "stop", unit, check=False)
        run("systemctl", "mask", "--runtime", unit, check=False)
    for path in (Path("/run/docker.sock"), Path("/run/containerd/containerd.sock")):
        if path.exists():
            os.chown(path, 0, 0)
            path.chmod(0o600)
    for process in ("dockerd", "containerd"):
        if run("pgrep", "-x", process, check=False).returncode == 0:
            raise RuntimeError("container daemon is still running")
    destination = Path("/etc/sudoers.d/zz-uv-network-policy")
    if destination.exists():
        raise RuntimeError("sudo policy already exists")
    policy = DIRECTORY / "sudoers"
    # GitHub's image can contain a readable runner grant with mode 0644.
    # Normalize only that root-owned regular file; keep global validation strict.
    normalize_runner_sudoers(Path("/etc/sudoers.d") / account.pw_name)
    policy.write_text(f"{account.pw_name} ALL=(ALL:ALL) !ALL\n")
    policy.chmod(0o440)
    run("visudo", "-cf", str(policy))
    shutil.copyfile(policy, destination)
    destination.chmod(0o440)
    run("visudo", "-c")
    if (
        run(
            "runuser", "-u", account.pw_name, "--", "sudo", "-n", "true", check=False
        ).returncode
        == 0
    ):
        raise RuntimeError("runner still has sudo access")


def install(profile, privileges, runner_uid, private_services):
    if os.geteuid() != 0 or runner_uid == 0 or not Path("/run/systemd/system").is_dir():
        raise RuntimeError("a non-root job on a systemd Linux VM is required")
    account = pwd.getpwuid(runner_uid)
    if not re.fullmatch(r"[a-z_][a-z0-9_-]*[$]?", account.pw_name):
        raise RuntimeError("unsupported runner account")
    source = Path(__file__).resolve().parent
    private_services = validate_private_origins(private_services)
    policies = json.loads((source / "policies.json").read_text())
    if profile not in policies["profiles"] or privileges not in {"drop", "retain"}:
        raise RuntimeError("unknown policy selection")
    if (
        DIRECTORY.exists()
        or run(
            "nft", "list", "table", "inet", TABLE, capture=True, check=False
        ).returncode
        == 0
    ):
        raise RuntimeError("network policy already installed")
    if privileges == "drop" and shutil.which("docker"):
        containers = run("docker", "ps", "--quiet", capture=True, check=False)
        if containers.returncode == 0 and containers.stdout.strip():
            raise RuntimeError("jobs with running containers are not supported")
    dns_servers = resolvers()
    connections = bootstrap_connections(runner_uid)
    run("useradd", "--system", "--no-create-home", "--shell", "/usr/sbin/nologin", USER)
    proxy_account = pwd.getpwnam(USER)
    DIRECTORY.mkdir(mode=0o755)
    for name in ("policy.py", "proxy.py", "policies.json"):
        shutil.copyfile(source / name, DIRECTORY / name)
        (DIRECTORY / name).chmod(0o644)
    audit = DIRECTORY / "audit"
    audit.mkdir(mode=0o755)
    os.chown(audit, proxy_account.pw_uid, proxy_account.pw_gid)
    (DIRECTORY / "settings.json").write_text(
        json.dumps(
            {
                "profile": profile,
                "resolvers": dns_servers,
                "privileges": privileges,
                "private_origins": private_services,
            }
        )
        + "\n"
    )
    unit = Path("/run/systemd/system") / SERVICE
    unit.write_text(f"""[Unit]
Description=Disposable CI network policy proxy
[Service]
Type=exec
User={USER}
ExecStart=/usr/bin/python3 -E -s {DIRECTORY}/proxy.py {DIRECTORY}
Restart=always
RestartSec=1
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
CapabilityBoundingSet=
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
ReadWritePaths={DIRECTORY}/audit
""")
    run("systemctl", "daemon-reload")
    print("Starting the root-owned policy service.", flush=True)
    run("systemctl", "start", SERVICE)
    for attempt in range(30):
        try:
            health()
            break
        except OSError:
            if attempt == 29:
                raise RuntimeError("proxy did not start") from None
            time.sleep(0.1)
    rules = DIRECTORY / "rules.nft"
    rules.write_text(
        firewall(
            proxy_account.pw_uid, runner_uid, dns_servers, connections, private_services
        )
    )
    run("nft", "--check", "--file", str(rules))
    print("Applying the default-deny firewall.", flush=True)
    run("nft", "--file", str(rules))
    # Once installed, the policy is deliberately not removed by action post
    # hooks. A disposable VM's trusted teardown owns that boundary.
    if privileges == "drop":
        print("Removing sudo and privileged container access.", flush=True)
        drop_privileges(account)
    health()
    print("Runner network policy is active.", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile")
    parser.add_argument("privileges", choices=("drop", "retain"))
    parser.add_argument("runner_uid", type=int)
    parser.add_argument("private_services", type=json.loads)
    args = parser.parse_args()
    install(args.profile, args.privileges, args.runner_uid, args.private_services)
