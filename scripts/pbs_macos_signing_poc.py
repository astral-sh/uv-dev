"""Compare signed PBS runtimes, installation relocation, and embedding linkage."""

import argparse
import hashlib
import json
import os
import platform
import plistlib
import secrets
import shlex
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import traceback
import urllib.request
from pathlib import Path

VERSION = "3.14.7"
BUILD = "20260901"
LIBRARY = "libpython3.14.dylib"
INPUTS = {
    "arm64": (
        "aarch64",
        "4632cb1a6edad9e73d3c81b6d2e69131637d995173e3e85005df14102b0592ba",
    ),
    "x86_64": (
        "x86_64",
        "7e151a7c9028855b61a7d6e78381f2020a1ef185281399f3b1aafc5e1c9a1a64",
    ),
}
FIXTURES = Path(__file__).with_name("pbs-macos-signing-poc")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class Experiment:
    def __init__(self, design: str, work: Path, reports: Path):
        self.design = design
        self.work = work
        self.reports = reports
        self.keychain = work / "identity.keychain-db"
        self.original_keychains = None
        self.identity = ""
        self.report = {
            "design": design,
            "machine": platform.machine(),
            "python": VERSION,
            "pbs_release": BUILD,
            "commands": [],
            "observations": {},
        }

    def run(self, arguments, *, cwd=None, env=None, check=True, private=False):
        command = [str(argument) for argument in arguments]
        if not private:
            print("+", " ".join(command), flush=True)
        result = subprocess.run(
            command,
            cwd=cwd or self.work,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=300,
            check=False,
        )
        if not private:
            print(result.stdout, end="", flush=True)
            self.report["commands"].append(
                {
                    "command": command,
                    "status": result.returncode,
                    "output": result.stdout,
                }
            )
        if check and result.returncode:
            label = "Private keychain setup" if private else " ".join(command)
            raise RuntimeError(f"{label} failed with status {result.returncode}")
        return result

    def create_identity(self):
        configuration = self.work / "identity.cnf"
        configuration.write_text(
            "[req]\nprompt = no\ndistinguished_name = subject\n"
            "x509_extensions = signing\n[subject]\nCN = PBS macOS experiment\n"
            "[signing]\nbasicConstraints = critical,CA:false\n"
            "keyUsage = critical,digitalSignature\n"
            "extendedKeyUsage = critical,codeSigning\n"
        )
        certificate = self.work / "identity.pem"
        private_key = self.work / "identity.key"
        self.run(
            [
                "openssl",
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-x509",
                "-days",
                "1",
                "-config",
                configuration,
                "-keyout",
                private_key,
                "-out",
                certificate,
            ],
            private=True,
        )
        password = secrets.token_hex(24)
        self.run(
            ["security", "create-keychain", "-p", password, self.keychain], private=True
        )
        self.run(
            ["security", "unlock-keychain", "-p", password, self.keychain], private=True
        )
        bundle = self.work / "identity.p12"
        self.run(
            [
                "openssl",
                "pkcs12",
                "-export",
                "-inkey",
                private_key,
                "-in",
                certificate,
                "-out",
                bundle,
                "-passout",
                f"pass:{password}",
            ],
            private=True,
        )
        self.run(
            [
                "security",
                "import",
                bundle,
                "-k",
                self.keychain,
                "-P",
                password,
                "-T",
                "/usr/bin/codesign",
                "-T",
                "/usr/bin/security",
            ],
            private=True,
        )
        self.run(
            [
                "security",
                "set-key-partition-list",
                "-S",
                "apple-tool:,apple:,codesign:",
                "-s",
                "-k",
                password,
                self.keychain,
            ],
            private=True,
        )
        certificate_der = self.work / "identity.der"
        self.run(
            [
                "openssl",
                "x509",
                "-in",
                certificate,
                "-outform",
                "DER",
                "-out",
                certificate_der,
            ]
        )
        self.identity = hashlib.sha1(certificate_der.read_bytes()).hexdigest()
        self.certificate_sha256 = sha256(certificate_der)
        self.report["certificate_sha256"] = self.certificate_sha256
        # codesign's private-key lookup uses the user search list even when its
        # certificate lookup is restricted with --keychain.
        self.original_keychains = shlex.split(
            self.run(["security", "list-keychains", "-d", "user"]).stdout
        )
        self.run(
            [
                "security",
                "list-keychains",
                "-d",
                "user",
                "-s",
                self.keychain,
                *self.original_keychains,
            ]
        )
        self.run(
            ["security", "find-identity", "-v", "-p", "codesigning", self.keychain]
        )

    def download(self):
        architecture, expected = INPUTS[platform.machine()]
        filename = f"cpython-{VERSION}+{BUILD}-{architecture}-apple-darwin-install_only_stripped.tar.gz"
        url = f"https://github.com/astral-sh/python-build-standalone/releases/download/{BUILD}/{filename}"
        archive = self.work / filename
        with urllib.request.urlopen(url, timeout=60) as source:
            archive.write_bytes(source.read())
        require(sha256(archive) == expected, "PBS archive checksum mismatch")
        self.report["input"] = {"url": url, "sha256": expected}
        destination = self.work / "prefix-a"
        destination.mkdir()
        with tarfile.open(archive) as source:
            source.extractall(destination, filter="data")
        return destination / "python"

    def native_files(self, runtime: Path):
        files = {}
        for path in sorted(runtime.rglob("*")):
            if path.is_symlink() or not path.is_file():
                continue
            with path.open("rb") as source:
                header = source.read(16)
            if header[:4] == b"\xcf\xfa\xed\xfe" and len(header) == 16:
                filetype = struct.unpack_from("<I", header, 12)[0]
                if filetype in {2, 6, 8}:
                    files[path.relative_to(runtime).as_posix()] = filetype
        require(bool(files), "No native code found")
        return files

    def manifest(self, runtime: Path):
        return {name: sha256(runtime / name) for name in self.native_files(runtime)}

    def sign(self, runtime: Path):
        entitlements = self.work / "entitlements.plist"
        entitlements.write_bytes(
            plistlib.dumps(
                {
                    "com.apple.security.cs.disable-library-validation": True,
                    "com.apple.security.cs.allow-jit": True,
                    "com.apple.security.cs.allow-unsigned-executable-memory": True,
                }
            )
        )
        native = self.native_files(runtime)
        for name, filetype in sorted(native.items(), key=lambda item: item[1] == 2):
            command = [
                "codesign",
                "--force",
                "--keychain",
                self.keychain,
                "--sign",
                self.identity,
                "--options",
                "runtime",
                "--timestamp=none",
            ]
            if filetype == 2:
                command.extend(["--entitlements", entitlements])
            self.run([*command, runtime / name])
            self.run(["codesign", "--verify", "--strict", runtime / name])
        self.report["native_file_count"] = len(native)

    def signature(self, path: Path, label: str):
        verification = self.run(["codesign", "--verify", "--strict", path], check=False)
        details = self.run(["codesign", "--display", "--verbose=4", path], check=False)
        prefix = self.reports / f"{label}-certificate-"
        self.run(
            ["codesign", "--display", f"--extract-certificates={prefix}", path],
            check=False,
        )
        leaf = Path(f"{prefix}0")
        result = {
            "valid": verification.returncode == 0,
            "certificate_sha256": sha256(leaf) if leaf.exists() else None,
            "details": details.stdout,
        }
        self.report["observations"][label] = result
        return result

    def create_stub(self, runtime: Path):
        library = runtime / "lib" / LIBRARY
        symbols = self.run(["nm", "-gUj", library]).stdout.splitlines()
        require(
            bool(symbols) and all(symbol.startswith("_") for symbol in symbols),
            "Unexpected symbol listing",
        )
        # A link-time stub supplies an installation-specific LC_LOAD_DYLIB path.
        # The runtime library and its embedded certificate remain untouched.
        content = (
            "--- !tapi-tbd-v3\n"
            f"archs: [ {platform.machine()} ]\nplatform: macosx\n"
            f"install-name: {json.dumps(str(library))}\n"
            "current-version: 3.14.0\ncompatibility-version: 3.14.0\n"
            f"exports:\n  - archs: [ {platform.machine()} ]\n"
            f"    symbols: {json.dumps(symbols)}\n...\n"
        )
        library.with_suffix(".tbd").write_text(content)
        (self.reports / f"{runtime.parent.name}.tbd").write_text(content)

    def smoke(self, runtime: Path):
        python = runtime / "bin/python3.14"
        code = (
            "import ctypes, ssl, sqlite3, tkinter; "
            "assert sqlite3.connect(':memory:').execute('select 42').fetchone() == (42,); "
            "assert ctypes.CFUNCTYPE(ctypes.c_int)(lambda: 42)() == 42; "
            "print(ssl.OPENSSL_VERSION, tkinter.Tcl().eval('info patchlevel'))"
        )
        self.run([python, "-I", "-c", code])
        environment = self.work / "venv"
        self.run(["uv", "venv", "--no-config", "--python", python, environment])
        self.run(
            [
                "uv",
                "pip",
                "install",
                "--no-config",
                "--python",
                environment / "bin/python",
                "--only-binary=:all:",
                "cffi==2.0.0",
                "pycparser==2.23",
            ]
        )
        self.run(
            [
                environment / "bin/python",
                "-I",
                "-c",
                "from cffi import FFI; callback = FFI().callback('int(void)', lambda: 42); assert callback() == 42",
            ]
        )

    def embed(
        self, runtime: Path, phase: str, *, discovered_python: Path | None = None
    ):
        destination = self.work / phase
        destination.mkdir()
        library_directory = runtime / "lib"
        executable = destination / "embed-c"
        command = [
            "clang",
            FIXTURES / "embedding.c",
            f"-I{runtime / 'include/python3.14'}",
            f"-L{library_directory}",
            "-lpython3.14",
            "-o",
            executable,
        ]
        if self.design == "rpath":
            self.run(command)
            negative = self.run([executable], check=False)
            require(
                negative.returncode != 0
                and "Library not loaded: @rpath/" in negative.stdout,
                "Embedding without an rpath must fail for the expected dyld reason",
            )
            command.append(f"-Wl,-rpath,{library_directory}")
        self.run(command)
        self.run(["otool", "-L", executable])
        self.run([executable])

        project = destination / "rust"
        shutil.copytree(FIXTURES / "embedding", project)
        configuration = destination / "pyo3-config.txt"
        configuration.write_text(
            "implementation=CPython\nversion=3.14\nshared=true\nabi3=false\n"
            f"lib_name=python3.14\nlib_dir={library_directory}\n"
            f"executable={runtime / 'bin/python3.14'}\npointer_width=64\n"
            "build_flags=\nsuppress_build_script_link_lines=false\n"
        )
        environment = dict(os.environ)
        if discovered_python is None:
            environment["PYO3_CONFIG_FILE"] = str(configuration)
        else:
            environment.pop("PYO3_CONFIG_FILE", None)
            environment["PYO3_PYTHON"] = str(discovered_python)
        environment.pop("RUSTFLAGS", None)
        if self.design == "rpath":
            environment["RUSTFLAGS"] = f"-C link-arg=-Wl,-rpath,{library_directory}"
        self.run(["cargo", "run", "--locked"], cwd=project, env=environment)
        self.run(["otool", "-L", project / "target/debug/pbs-embedding-poc"])

    def observe_uv_install(self, archive: Path, expected: dict):
        architecture = INPUTS[platform.machine()][0]
        metadata = self.work / "downloads.json"
        metadata.write_text(
            json.dumps(
                {
                    f"cpython-{VERSION}-darwin-{architecture}-none": {
                        "name": "cpython",
                        "arch": {"family": architecture, "variant": None},
                        "os": "darwin",
                        "libc": "none",
                        "major": 3,
                        "minor": 14,
                        "patch": 7,
                        "prerelease": "",
                        "url": archive.as_uri(),
                        "sha256": sha256(archive),
                        "variant": None,
                        "build": BUILD,
                    }
                }
            )
        )
        installation = self.work / "uv-installed"
        self.run(
            [
                "uv",
                "python",
                "install",
                VERSION,
                "--no-config",
                "--no-bin",
                "--install-dir",
                installation,
                "--python-downloads-json-url",
                metadata.as_uri(),
            ]
        )
        [runtime] = list(installation.glob(f"cpython-{VERSION}-*"))
        actual = self.manifest(runtime)
        changed = [name for name in expected if expected[name] != actual.get(name)]
        self.report["observations"]["uv_install_changed_native_files"] = changed
        require(
            changed == [f"lib/{LIBRARY}"],
            "Unexpected native mutations during uv installation",
        )
        interpreter = self.signature(
            runtime / "bin/python3.14", "uv-installed-interpreter"
        )
        require(
            interpreter["valid"]
            and interpreter["certificate_sha256"] == self.certificate_sha256,
            "uv installation changed the interpreter's signature",
        )
        library = self.signature(runtime / "lib" / LIBRARY, "uv-installed-library")
        require(
            not (
                library["valid"]
                and library["certificate_sha256"] == self.certificate_sha256
            ),
            "A modified library unexpectedly retained its valid original signature",
        )
        return runtime

    def discover_installed_stub(
        self, runtime: Path, signed_library: Path, expected: dict
    ):
        # Restoring the exact signed bytes models an installer which omits the
        # install-name rewrite. uv still supplies its real installation metadata.
        replacement = runtime / "lib" / f"{LIBRARY}.replacement"
        shutil.copy2(signed_library, replacement)
        replacement.replace(runtime / "lib" / LIBRARY)
        self.create_stub(runtime)
        environment = self.work / "managed-venv"
        self.run(
            ["uv", "venv", "--no-config", "--python", VERSION, environment],
            env=dict(os.environ, UV_PYTHON_INSTALL_DIR=str(runtime.parent)),
        )
        python = environment / "bin/python"
        self.run(
            [
                python,
                "-I",
                "-c",
                "import sys, sysconfig; print(sys.executable, sys.base_prefix, sysconfig.get_config_var('LIBDIR'))",
            ]
        )
        self.embed(runtime, "embedding-uv-discovery", discovered_python=python)
        require(
            self.manifest(runtime) == expected,
            "Discovered embedding changed native runtime bytes",
        )
        signature = self.signature(runtime / "lib" / LIBRARY, "uv-stub-library")
        require(
            signature["valid"]
            and signature["certificate_sha256"] == self.certificate_sha256,
            "The stub experiment lost the original installed library signature",
        )
        self.report["observations"]["pyo3_automatic_discovery"] = True

    def execute(self):
        self.run(["sw_vers"])
        self.run(["xcrun", "--show-sdk-version"])
        self.run(["uv", "--version"])
        runtime = self.download()
        library = runtime / "lib" / LIBRARY
        self.run(["otool", "-L", runtime / "bin/python3.14"])
        self.run(["otool", "-L", library])
        if self.design == "rpath":
            self.run(["install_name_tool", "-id", f"@rpath/{LIBRARY}", library])
        self.create_identity()
        self.sign(runtime)
        expected = self.manifest(runtime)
        self.report["signed_manifest"] = expected
        original = self.signature(library, "original-library")
        require(
            original["valid"]
            and original["certificate_sha256"] == self.certificate_sha256,
            "Expected the disposable signing certificate",
        )
        archive = self.work / "signed-python.tar.gz"
        with tarfile.open(archive, "w:gz") as destination:
            destination.add(runtime, arcname="python")
        installed = self.observe_uv_install(archive, expected)
        if self.design == "stub":
            self.discover_installed_stub(installed, library, expected)

        if self.design == "rewrite":
            self.run(["install_name_tool", "-id", library, library])
            self.signature(library, "rewritten-library")
            self.run(["codesign", "--force", "--sign", "-", library])
            replacement = self.signature(library, "adhoc-library")
            require(
                replacement["valid"] and replacement["certificate_sha256"] is None,
                "Expected a valid ad-hoc library signature",
            )
        elif self.design == "stub":
            self.create_stub(runtime)

        self.smoke(runtime)
        self.embed(runtime, "embedding-a")
        if self.design != "rewrite":
            require(self.manifest(runtime) == expected, "Native runtime bytes changed")
            relocated = self.work / "prefix-b/python"
            shutil.copytree(runtime, relocated, symlinks=True)
            runtime.parent.rename(self.work / "unavailable-prefix-a")
            if self.design == "stub":
                self.create_stub(relocated)
            self.run(
                [
                    relocated / "bin/python3.14",
                    "-I",
                    "-c",
                    "import ssl, sqlite3; print('relocated runtime works')",
                ]
            )
            self.embed(relocated, "embedding-b")
            require(
                self.manifest(relocated) == expected,
                "Relocation changed native runtime bytes",
            )
            signature = self.signature(relocated / "lib" / LIBRARY, "relocated-library")
            require(
                signature["valid"]
                and signature["certificate_sha256"] == self.certificate_sha256,
                "Relocation lost the original library signature",
            )
        self.report["success"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("design", choices=("rewrite", "rpath", "stub"))
    parser.add_argument("--report-directory", type=Path, required=True)
    arguments = parser.parse_args()
    require(sys.platform == "darwin", "These experiments require macOS")
    reports = arguments.report_directory.resolve()
    reports.mkdir(parents=True)
    scratch = Path.home() / "code/tmp"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="pbs-macos-signing-", dir=scratch
    ) as temporary:
        experiment = Experiment(arguments.design, Path(temporary), reports)
        try:
            experiment.execute()
        except Exception:
            experiment.report["success"] = False
            experiment.report["error"] = traceback.format_exc()
            raise
        finally:
            if experiment.original_keychains is not None:
                subprocess.run(
                    [
                        "security",
                        "list-keychains",
                        "-d",
                        "user",
                        "-s",
                        *experiment.original_keychains,
                    ],
                    check=False,
                )
            if experiment.keychain.exists():
                subprocess.run(
                    ["security", "delete-keychain", str(experiment.keychain)],
                    check=False,
                )
            (reports / "report.json").write_text(
                json.dumps(experiment.report, indent=2) + "\n"
            )
            print(
                "POC_RESULT",
                json.dumps(
                    {
                        "design": arguments.design,
                        "machine": platform.machine(),
                        "success": experiment.report.get("success", False),
                        "observations": experiment.report["observations"],
                    }
                ),
                flush=True,
            )


if __name__ == "__main__":
    main()
