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

from certificates import generate
from policy import load, validate_private_origins
from url_settings import compile_policy

DIRECTORY = Path("/run/uv-network-policy")
USER = "uv-network-policy"
TABLE = "uv_network_policy"
SERVICE = "uv-network-policy.service"
URL_CA = Path("/usr/local/share/ca-certificates/uv-network-policy.crt")


def run(*arguments, capture=False, check=True):
    return subprocess.run(
        arguments,
        check=check,
        text=True,
        stdout=subprocess.PIPE if capture else subprocess.DEVNULL,
        stderr=subprocess.PIPE if capture else None,
    )


def write_trusted(path, contents, mode=0o644):
    """Create a policy file without a writable window or replacing another file."""
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    with os.fdopen(descriptor, "w", encoding="utf-8") as destination:
        os.fchmod(destination.fileno(), mode)
        destination.write(contents)


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
    private_services = validate_private_origins(private_services or {})
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
        # Conntrack can reuse a tuple after a new handshake. Preserve packets
        # from the trusted bootstrap flow, never a later SYN using its ports.
        rules.append(
            f"meta skuid {runner_uid} ct state established tcp flags & syn == 0 {family} daddr {ipaddress.ip_address(address)} tcp sport {int(source_port)} tcp dport {int(port)} accept"
        )
    exceptions = "\n    ".join(rules)
    redirects = "\n    ".join(
        f"ip daddr {authority.split(':')[0]} tcp dport {authority.split(':')[1]} redirect to :18080"
        for authority in private_services
    )
    private_ports = sorted({authority.split(":")[1] for authority in private_services})
    # Exact injected addresses are translated before filtering. Reject other
    # routes to those host-local services, including loopback and bridge aliases.
    private_reject = (
        f"tcp dport {{ {', '.join(private_ports)} }} reject with tcp reset"
        if private_ports
        else ""
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
    {private_reject}
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
    {private_reject}
    iifname != \"lo\" tcp dport {{ 1053, 18080, 18443 }} reject
    iifname != \"lo\" udp dport 1053 reject
  }}
}}
"""


def health():
    with socket.create_connection(("127.0.0.1", 18080), timeout=2) as stream:
        stream.sendall(
            b"GET /__uv_network_proxy_health HTTP/1.1\r\nHost: localhost:18080\r\nConnection: close\r\n\r\n"
        )
        if b" 204 " not in stream.recv(1024).split(b"\r\n", 1)[0]:
            raise RuntimeError("proxy health check failed")


def normalize_runner_sudoers(path):
    if path.exists():
        metadata = path.lstat()
        if metadata.st_uid != 0 or not stat.S_ISREG(metadata.st_mode):
            raise RuntimeError("unexpected runner sudoers file")
        path.chmod(0o440)


def create_proxy_account():
    # The URL-mode leaf key is readable by this primary group. Do not inherit
    # an image's potentially shared default useradd group.
    run(
        "useradd",
        "--system",
        "--user-group",
        "--no-create-home",
        "--shell",
        "/usr/sbin/nologin",
        USER,
    )
    return pwd.getpwnam(USER)


def restrict_sudo(path):
    """Make the canonical root-owned sudo inode inaccessible to non-root users."""
    path = path.resolve(strict=True)
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        before = os.fstat(descriptor)
        if (
            before.st_uid != 0
            or not stat.S_ISREG(before.st_mode)
            or stat.S_IMODE(before.st_mode) & 0o022
        ):
            raise RuntimeError("unexpected sudo executable")
        os.fchmod(descriptor, 0o700)
        after = os.fstat(descriptor)
        current = path.stat()
        if (
            (current.st_dev, current.st_ino) != (before.st_dev, before.st_ino)
            or after.st_uid != 0
            or stat.S_IMODE(after.st_mode) != 0o700
        ):
            raise RuntimeError("sudo executable restriction failed")
    finally:
        os.close(descriptor)
    return path


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
    write_trusted(policy, f"{account.pw_name} ALL=(ALL:ALL) !ALL\n", 0o440)
    run("visudo", "-cf", str(policy))
    write_trusted(destination, policy.read_text(), 0o440)
    run("visudo", "-c")
    # A later command-specific sudoers grant can override our deny rule while
    # still denying `sudo true`. Remove executable access independently of
    # sudoers ordering on the reviewed Ubuntu/Debian images.
    configured_sudo = shutil.which("sudo")
    canonical_sudo = Path("/usr/bin/sudo")
    if configured_sudo is None or not Path(configured_sudo).samefile(canonical_sudo):
        raise RuntimeError("unexpected sudo executable")
    canonical_sudo = restrict_sudo(canonical_sudo)
    if (
        run(
            "runuser",
            "-u",
            account.pw_name,
            "--",
            str(canonical_sudo),
            "-n",
            "true",
            check=False,
        ).returncode
        == 0
    ):
        raise RuntimeError("runner still has sudo access")


def install(
    profile,
    privileges,
    runner_uid,
    private_services,
    url_profile="",
    runtime_services=None,
):
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
    url_policy = (
        compile_policy(
            source / "url-policies.json",
            url_profile,
            load(source / "policies.json", profile),
            runtime_services or {},
        )
        if url_profile
        else None
    )
    if url_policy and URL_CA.exists():
        raise RuntimeError("URL policy certificate already installed")
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
    proxy_account = create_proxy_account()
    DIRECTORY.mkdir(mode=0o755)
    DIRECTORY.chmod(0o755)
    for name in (
        "policy.py",
        "proxy.py",
        "policies.json",
        "url_policy.py",
        "url_proxy.py",
    ):
        write_trusted(DIRECTORY / name, (source / name).read_text())
    audit = DIRECTORY / "audit"
    audit.mkdir(mode=0o755)
    os.chown(audit, proxy_account.pw_uid, proxy_account.pw_gid)
    if url_policy:
        write_trusted(
            DIRECTORY / "url-policy.json", json.dumps(url_policy.to_dict()) + "\n"
        )
        certificate, _, server_key = generate(DIRECTORY, url_policy.hosts)
        os.chown(server_key, 0, proxy_account.pw_gid)
        server_key.chmod(0o640)
        write_trusted(URL_CA, certificate.read_text())
        run("update-ca-certificates")
        # The already-running .NET worker rechecks Linux system-root timestamps
        # at five-second intervals. Let it observe the CA before TLS cutover.
        time.sleep(5.1)
    write_trusted(
        DIRECTORY / "settings.json",
        json.dumps(
            {
                "profile": profile,
                "resolvers": dns_servers,
                "privileges": privileges,
                "private_origins": private_services,
                "url_profile": url_profile,
            }
        )
        + "\n",
    )
    unit = Path("/run/systemd/system") / SERVICE
    write_trusted(
        unit,
        f"""[Unit]
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
""",
    )
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
    write_trusted(
        rules,
        firewall(
            proxy_account.pw_uid, runner_uid, dns_servers, connections, private_services
        ),
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
    parser.add_argument("--url-profile", default="")
    parser.add_argument("--runtime-services", type=json.loads, default={})
    args = parser.parse_args()
    install(
        args.profile,
        args.privileges,
        args.runner_uid,
        args.private_services,
        args.url_profile,
        args.runtime_services,
    )
