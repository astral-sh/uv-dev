"""Exercise certificate generation without changing system trust or networking."""

import hashlib
import io
import re
import ssl
import stat
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
ACTION = ROOT / ".github/actions/runner-network-policy"
sys.path.insert(0, str(ACTION))

import certificates


def openssl(*arguments, check=True):
    return subprocess.run(
        ["openssl", *map(str, arguments)],
        check=check,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )


def extension(certificate, name):
    lines = (
        openssl("x509", "-in", certificate, "-noout", "-ext", name)
        .stdout.decode("ascii")
        .splitlines()
    )
    return lines[0].strip(), " ".join(line.strip() for line in lines[1:])


class CertificateTests(unittest.TestCase):
    def setUp(self):
        scratch = Path.home() / "code/tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(
            prefix="uv-runner-url-certificates-", dir=scratch
        )
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)

    def test_exact_server_certificate_and_private_permissions(self):
        names = ("artifactcache.actions.githubusercontent.com", "api.github.com")
        run = certificates._run
        observed = []

        def inspect(*arguments):
            if "-keyout" in arguments:
                key = Path(arguments[arguments.index("-keyout") + 1])
                observed.append(key.name)
                self.assertEqual(stat.S_IMODE(key.stat().st_mode), 0o600)
                self.assertEqual(stat.S_IMODE(key.parent.stat().st_mode), 0o700)
            run(*arguments)
            if "-keyout" in arguments:
                self.assertEqual(stat.S_IMODE(key.stat().st_mode), 0o600)

        output, errors = io.StringIO(), io.StringIO()
        with (
            patch.object(certificates, "_run", side_effect=inspect),
            redirect_stdout(output),
            redirect_stderr(errors),
        ):
            ca_certificate, server_certificate, server_key = certificates.generate(
                self.directory, names
            )
        self.assertEqual((output.getvalue(), errors.getvalue()), ("", ""))
        self.assertEqual(observed, ["ca.key", "server.key"])
        self.assertEqual(
            (ca_certificate, server_certificate, server_key),
            tuple(
                self.directory / name for name in ("ca.crt", "server.crt", "server.key")
            ),
        )
        self.assertEqual(
            sorted(path.name for path in self.directory.iterdir()),
            ["ca.crt", "server.crt", "server.key"],
        )
        self.assertEqual(stat.S_IMODE(ca_certificate.stat().st_mode), 0o644)
        self.assertEqual(stat.S_IMODE(server_certificate.stat().st_mode), 0o644)
        self.assertEqual(stat.S_IMODE(server_key.stat().st_mode), 0o600)
        self.assertEqual(
            extension(ca_certificate, "basicConstraints"),
            ("X509v3 Basic Constraints: critical", "CA:TRUE, pathlen:0"),
        )
        self.assertEqual(
            extension(ca_certificate, "keyUsage"),
            ("X509v3 Key Usage: critical", "Certificate Sign, CRL Sign"),
        )
        self.assertEqual(
            extension(server_certificate, "basicConstraints"),
            ("X509v3 Basic Constraints: critical", "CA:FALSE"),
        )
        self.assertEqual(
            extension(server_certificate, "extendedKeyUsage"),
            ("X509v3 Extended Key Usage:", "TLS Web Server Authentication"),
        )
        self.assertEqual(
            tuple(
                re.findall(
                    r"DNS:([^,\s]+)", extension(server_certificate, "subjectAltName")[1]
                )
            ),
            names,
        )
        for certificate in (ca_certificate, server_certificate):
            with self.subTest(certificate=certificate.name):
                validity = dict(
                    line.split("=", 1)
                    for line in openssl("x509", "-in", certificate, "-noout", "-dates")
                    .stdout.decode("ascii")
                    .splitlines()
                )
                self.assertEqual(
                    ssl.cert_time_to_seconds(validity["notAfter"])
                    - ssl.cert_time_to_seconds(validity["notBefore"]),
                    24 * 60 * 60,
                )
        for name in names:
            with self.subTest(hostname=name):
                openssl(
                    "verify",
                    "-x509_strict",
                    "-CAfile",
                    ca_certificate,
                    "-purpose",
                    "sslserver",
                    "-verify_hostname",
                    name,
                    server_certificate,
                )
        self.assertNotEqual(
            openssl(
                "verify",
                "-CAfile",
                ca_certificate,
                "-verify_hostname",
                "not-permitted.example",
                server_certificate,
                check=False,
            ).returncode,
            0,
        )
        self.assertEqual(
            hashlib.sha256(
                openssl("x509", "-in", server_certificate, "-pubkey", "-noout").stdout
            ).digest(),
            hashlib.sha256(
                openssl("pkey", "-in", server_key, "-pubout").stdout
            ).digest(),
        )

    def test_normalizes_exact_hosts_without_adding_wildcards(self):
        _, server_certificate, _ = certificates.generate(
            self.directory, ("API.GITHUB.COM.", "api.github.com")
        )
        self.assertEqual(
            extension(server_certificate, "subjectAltName"),
            ("X509v3 Subject Alternative Name:", "DNS:api.github.com"),
        )

    def test_rejects_invalid_names_before_creating_files(self):
        for names in (
            (),
            ("*.github.com",),
            ("127.0.0.1",),
            ("[::1]",),
            ("api.github.com\nDNS.2=attacker.example",),
            ("api.github.com, DNS:attacker.example",),
            tuple(f"host-{index}.example" for index in range(129)),
        ):
            with self.subTest(hosts=names), self.assertRaises(ValueError):
                certificates.generate(self.directory, names)
            self.assertEqual(list(self.directory.iterdir()), [])

    def test_removes_real_ca_key_when_leaf_generation_fails(self):
        run = certificates._run
        ca_key = None

        def fail_after_ca(*arguments):
            nonlocal ca_key
            if ca_key is not None:
                self.assertGreater(ca_key.stat().st_size, 0)
                raise RuntimeError("synthetic certificate failure")
            ca_key = Path(arguments[arguments.index("-keyout") + 1])
            run(*arguments)

        with (
            patch.object(certificates, "_run", side_effect=fail_after_ca),
            self.assertRaisesRegex(RuntimeError, "^synthetic certificate failure$"),
        ):
            certificates.generate(self.directory, ("api.github.com",))
        self.assertIsNotNone(ca_key)
        self.assertFalse(ca_key.exists())
        self.assertEqual(list(self.directory.iterdir()), [])

    def test_does_not_replace_existing_output(self):
        existing = self.directory / "server.key"
        existing.write_bytes(b"existing synthetic key")
        with self.assertRaisesRegex(
            FileExistsError, "^certificate output already exists$"
        ):
            certificates.generate(self.directory, ("api.github.com",))
        self.assertEqual(existing.read_bytes(), b"existing synthetic key")
        self.assertEqual(list(self.directory.iterdir()), [existing])

    def test_openssl_failure_does_not_expose_command_output(self):
        error = subprocess.CalledProcessError(
            1,
            ["openssl", "synthetic-secret"],
            output=b"synthetic-secret",
            stderr=b"synthetic-secret",
        )
        with (
            patch.object(certificates.subprocess, "run", side_effect=error) as run,
            self.assertRaisesRegex(RuntimeError, "^certificate generation failed$"),
        ):
            certificates._run("synthetic-secret")
        self.assertEqual(
            run.call_args.kwargs,
            {
                "check": True,
                "stdout": subprocess.DEVNULL,
                "stderr": subprocess.DEVNULL,
            },
        )


if __name__ == "__main__":
    unittest.main()
