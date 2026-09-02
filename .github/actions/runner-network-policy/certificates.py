"""Create one short-lived certificate for the policy's exact HTTPS hosts."""

import os
import secrets
import subprocess
import tempfile
from pathlib import Path

from policy import hostname

CA_CONFIG = """[req]
prompt=no
distinguished_name=dn
x509_extensions=ca
[dn]
CN=uv runner network policy CA
[ca]
basicConstraints=critical,CA:TRUE,pathlen:0
keyUsage=critical,keyCertSign,cRLSign
subjectKeyIdentifier=hash
"""


def _run(*arguments: str | Path) -> None:
    try:
        subprocess.run(
            ["openssl", *map(str, arguments)],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except (OSError, subprocess.CalledProcessError):
        raise RuntimeError("certificate generation failed") from None


def _private_file(path: Path, contents: str = "") -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="ascii") as destination:
        destination.write(contents)


def generate(directory: Path, hosts: tuple[str, ...]) -> tuple[Path, Path, Path]:
    """Publish certificates into an existing trusted directory without replacing files."""
    names = tuple(dict.fromkeys(hostname(value) for value in hosts))
    if not 1 <= len(names) <= 128 or any(len(value) > 253 for value in names):
        raise ValueError("exact TLS hostnames are required")

    ca_certificate = directory / "ca.crt"
    server_certificate = directory / "server.crt"
    server_key = directory / "server.key"
    outputs = (ca_certificate, server_certificate, server_key)
    if any(path.exists() or path.is_symlink() for path in outputs):
        raise FileExistsError("certificate output already exists")

    with tempfile.TemporaryDirectory(
        prefix=".uv-network-certificates-", dir=directory
    ) as temporary:
        staging = Path(temporary)
        ca_key = staging / "ca.key"
        published: list[Path] = []
        complete = False
        try:
            _private_file(ca_key)
            _private_file(staging / "server.key")
            _private_file(staging / "ca.cnf", CA_CONFIG)
            _private_file(
                staging / "server.cnf",
                "[req]\nprompt=no\ndistinguished_name=dn\n"
                "req_extensions=request\n[dn]\nCN="
                + names[0]
                + "\n[request]\nsubjectAltName=@names\n[server]\n"
                "basicConstraints=critical,CA:FALSE\n"
                "keyUsage=critical,digitalSignature,keyEncipherment\n"
                "extendedKeyUsage=serverAuth\n"
                "subjectKeyIdentifier=hash\n"
                "authorityKeyIdentifier=keyid:always\n"
                "subjectAltName=@names\n[names]\n"
                + "".join(
                    f"DNS.{index}={name}\n" for index, name in enumerate(names, start=1)
                ),
            )
            _run(
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-days",
                "1",
                "-set_serial",
                hex(secrets.randbits(159) or 1),
                "-config",
                staging / "ca.cnf",
                "-keyout",
                ca_key,
                "-out",
                staging / "ca.crt",
            )
            _run(
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-config",
                staging / "server.cnf",
                "-keyout",
                staging / "server.key",
                "-out",
                staging / "server.csr",
            )
            _run(
                "x509",
                "-req",
                "-sha256",
                "-days",
                "1",
                "-in",
                staging / "server.csr",
                "-CA",
                staging / "ca.crt",
                "-CAkey",
                ca_key,
                "-set_serial",
                hex(secrets.randbits(159) or 1),
                "-extfile",
                staging / "server.cnf",
                "-extensions",
                "server",
                "-out",
                staging / "server.crt",
            )
            for source, destination, mode in (
                (staging / "ca.crt", ca_certificate, 0o644),
                (staging / "server.crt", server_certificate, 0o644),
                (staging / "server.key", server_key, 0o600),
            ):
                source.chmod(mode)
                # A same-filesystem hard link publishes the existing permissions
                # atomically and refuses to replace an existing file or symlink.
                os.link(source, destination)
                published.append(destination)
            complete = True
        finally:
            try:
                ca_key.unlink(missing_ok=True)
            finally:
                if not complete:
                    for path in reversed(published):
                        path.unlink(missing_ok=True)

    return outputs
