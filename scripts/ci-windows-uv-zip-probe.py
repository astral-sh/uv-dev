"""Time each HTTPS hop and local ZIP operation on a Windows CI runner."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
from urllib.parse import urlsplit

ZIP = "uv-x86_64-pc-windows-msvc.zip"
RELEASE = f"uv/releases/download/0.13.6/{ZIP}"
URLS = {
    "astral": f"https://releases.astral.sh/github/{RELEASE}",
    "github": f"https://github.com/astral-sh/{RELEASE}",
}
SIZE = 15_239_500
SHA256 = "a2a8820d5eea32f08ef467ef3b7f66883a9b076d5737a2fc2fbb5f458d1bb50f"
CURL = "curl.exe" if os.name == "nt" else "curl"


def emit(kind, record):
    print(f"{kind} {json.dumps(record, separators=(',', ':'))}", flush=True)


def fetch(source, destination, budget):
    """Use separate GETs per redirect so every timing belongs to exactly one host."""
    start = time.monotonic()
    url = URLS[source]
    hops = []
    for _ in range(8):
        remaining = budget - (time.monotonic() - start)
        if remaining <= 0:
            break
        run = subprocess.run(
            [CURL, "--disable", "--silent", "--show-error", "--http1.1",
             "--proto", "=https", "--connect-timeout", str(min(60, remaining)),
             "--max-time", str(remaining), "--output", str(destination),
             "--write-out", "%{json}", url],
            capture_output=True,
        )
        # curl's full JSON contains signed redirect URLs. Persist only timing fields.
        raw = json.loads(run.stdout)
        ts = {key: raw["time_" + key] * 1000 for key in
              ("namelookup", "connect", "appconnect", "pretransfer", "starttransfer", "total")}
        # An unset milestone (zero) on a failed request has no measurable interval.
        def elapsed(end, begin=None):
            if not ts[end] or (begin and not ts[begin]):
                return None
            return round(ts[end] - (ts[begin] if begin else 0), 2)

        hop = {
            "host": urlsplit(url).hostname, "http": raw["http_code"],
            "curl_exit": run.returncode, "bytes": raw["size_download"],
            "dns_ms": elapsed("namelookup"), "tcp_ms": elapsed("connect", "namelookup"),
            "tls_ms": elapsed("appconnect", "connect"),
            "request_setup_ms": elapsed("pretransfer", "appconnect"),
            "wait_first_byte_ms": elapsed("starttransfer", "pretransfer"),
            "body_ms": elapsed("total", "starttransfer"), "total_ms": elapsed("total"),
        }
        hops.append(hop)
        emit("ZIP_HOP", {"source": source, **hop})
        if run.returncode or not 300 <= raw["http_code"] < 400:
            break
        url = raw.get("redirect_url", "")
        if urlsplit(url).scheme != "https":
            break
    return {"source": source, "wall_s": round(time.monotonic() - start, 4), "hops": hops}


def main():
    sample = int(os.environ["ZIP_SAMPLE"])
    order = ["astral", "github"] if sample % 2 else ["github", "astral"]
    root = Path(os.environ["RUNNER_TEMP"])
    results = {
        "sample": sample, "runner": os.environ.get("RUNNER_NAME"), "order": order,
        "curl": subprocess.check_output([CURL, "--version"], text=True).splitlines()[0],
        "proxy_env_configured": any(os.environ.get(k) for k in
                                    ("HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy")),
        "files": [], "discard": [],
    }
    with tempfile.TemporaryDirectory(dir=root) as directory:
        temp = Path(directory)
        for source in order:
            archive = temp / f"{source}.zip"
            result = fetch(source, archive, 360)
            last = result["hops"][-1]
            result["validated"] = False
            if not last["curl_exit"] and last["http"] == 200:
                start = time.monotonic()
                with archive.open("rb") as reader:
                    digest = hashlib.sha256()
                    for chunk in iter(lambda: reader.read(1024 * 1024), b""):
                        digest.update(chunk)
                result["hash_s"] = round(time.monotonic() - start, 5)
                result["sha256"] = digest.hexdigest()
                if result["sha256"] == SHA256 and archive.stat().st_size == SIZE:
                    target = temp / source
                    target.mkdir()
                    tar = Path(os.environ["SYSTEMROOT"]) / "System32/tar.exe" if os.name == "nt" else "tar"
                    start = time.monotonic()
                    extracted = subprocess.run([tar, "-xf", archive, "-C", target], capture_output=True)
                    result["extract_s"] = round(time.monotonic() - start, 5)
                    result["validated"] = extracted.returncode == 0 and (target / "uv.exe").is_file()
            results["files"].append(result)
        # Repeat a slow or failed transfer without writing the ZIP, while the runner is still here.
        for result in reversed(results["files"]):
            if result["wall_s"] >= 10 or not result["validated"]:
                results["discard"].append(fetch(result["source"], os.devnull, 90))
    output = root / "uv-zip-result.json"
    output.write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
    emit("ZIP_RESULT", results)
    if not all(result["validated"] for result in results["files"]):
        raise SystemExit("One or both ZIPs failed verification. See the recorded HTTP and curl status.")


if __name__ == "__main__":
    main()
