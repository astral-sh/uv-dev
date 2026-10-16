"""Apply the same capture hooks to two pinned interpreter-cache implementations."""

import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path

BASELINE = "b757f15552203f42d8081b2378955e725cd732ce"
PROPOSAL = "9b7089394f4c3b00e0d1f0a8636588a6c32dbd6a"
HERE = Path(__file__).resolve().parent


def replace_once(text, before, after):
    if text.count(before) != 1:
        raise ValueError(f"Expected one source anchor: {before!r}")
    return text.replace(before, after, 1)


def prepare(root, revision, proposal_source):
    actual = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=root, text=True
    ).strip()
    if actual != revision or revision not in (BASELINE, PROPOSAL):
        raise ValueError("Source revision does not match the fixed comparison")
    path = root / "crates/uv-python-interpreter/src/interpreter.rs"
    original = path.read_text(encoding="utf-8")
    text = '#[path = "interpreter_capture.rs"]\nmod interpreter_capture;\n\n' + original
    entry = "Some(&cache_entry)" if revision == BASELINE else "cache_entry.as_ref()"
    anchor = "        // Read from the cache.\n"
    text = replace_once(
        text,
        anchor,
        f'        interpreter_capture::record("selected", executable, {entry}, None, Some(&modified));\n\n'
        + anchor,
    )
    anchor = "                            return Ok(cached.data);"
    hit_entry = "Some(&cache_entry)" if revision == BASELINE else "Some(cache_entry)"
    text = replace_once(
        text,
        anchor,
        f'                            interpreter_capture::record("hit", executable, {hit_entry}, Some(&cached.data), Some(&modified));\n'
        + anchor,
    )
    anchor = "        let output = command.output().map_err(|err| {"
    text = replace_once(
        text,
        anchor,
        '        interpreter_capture::record("spawn", interpreter, None, None, None);\n'
        + anchor,
    )
    anchor = "        let info = Self::query(executable, cache)?;"
    text = replace_once(
        text,
        anchor,
        anchor
        + f'\n        interpreter_capture::record("queried", executable, {entry}, Some(&info), Some(&modified));',
    )
    anchor = "                    data: &info,\n                })?,\n            )?;"
    text = replace_once(
        text,
        anchor,
        anchor
        + '\n            interpreter_capture::record("published", executable, Some(&cache_entry), Some(&info), Some(&modified));',
    )
    path.write_text(text, encoding="utf-8")
    shutil.copyfile(HERE / "trace.rs", path.with_name("interpreter_capture.rs"))
    example = root / "crates/uv-python-interpreter/examples/interpreter-cache-probe.rs"
    example.parent.mkdir(exist_ok=True)
    shutil.copyfile(HERE / "probe.rs", example)

    if revision == BASELINE:
        proposal = (proposal_source / "crates/uv/tests/project/run.rs").read_text(
            encoding="utf-8"
        )
        start = proposal.index(
            "#[test]\nfn run_recreated_virtualenv_uses_current_interpreter"
        )
        end = proposal.index("\n#[test]", start + 1)
        test_file = root / "crates/uv/tests/project/run.rs"
        text = test_file.read_text(encoding="utf-8")
        position = text.index("#[test]")
        test_file.write_text(
            text[:position] + proposal[start:end] + "\n" + text[position:],
            encoding="utf-8",
        )

    sync = root / "crates/uv/tests/sync/sync.rs"
    text = sync.read_text(encoding="utf-8")
    text = replace_once(
        text,
        '        !stderr.contains("Would replace existing virtual environment"),',
        '        !stderr.contains("Would replace project environment"),',
    )
    text = replace_once(
        text,
        '    let stderr = String::from_utf8_lossy(&output.stderr);\n    assert!(\n        !stderr.contains("Would replace project environment"),',
        '    let stderr = String::from_utf8_lossy(&output.stderr);\n    assert!(output.status.success(), "{stderr}");\n    assert!(\n        !stderr.contains("Would replace project environment"),',
    )
    text = replace_once(
        text,
        "use uv_test::{TestContext, download_to_disk, uv_snapshot, venv_bin_path};",
        "use uv_test::{TestContext, download_to_disk, run_and_format, uv_snapshot, venv_bin_path};",
    )
    text = replace_once(
        text,
        '    let output = context.sync().arg("--dry-run").arg("-vv").output()?;',
        '    let (_, output) = run_and_format(\n        context.sync().arg("--dry-run").arg("-vv"),\n        context.filters(),\n        "sync_dry_run_capture",\n        None,\n        None,\n    );',
    )
    sync.write_text(text, encoding="utf-8")
    helper = root / "crates/uv-test/src/lib.rs"
    text = helper.read_text(encoding="utf-8")
    anchor = "        if let Some(rust_log) = env::var_os(EnvVars::UV_INTERNAL__TEST_RUST_LOG) {\n            command.env(EnvVars::RUST_LOG, rust_log);\n        }"
    text = replace_once(
        text, anchor, anchor + '\n        command.env(EnvVars::UV_HTTP_RETRIES, "0");'
    )
    helper.write_text(text, encoding="utf-8")
    subprocess.run(
        [
            "git",
            "add",
            "-N",
            "--",
            "crates/uv-python-interpreter/src/interpreter_capture.rs",
            "crates/uv-python-interpreter/examples/interpreter-cache-probe.rs",
        ],
        cwd=root,
        check=True,
        timeout=30,
    )
    subprocess.run(
        ["rustfmt", "--edition", "2024", str(path), str(example), str(sync)],
        cwd=root,
        check=True,
        timeout=30,
    )
    return {
        "revision": revision,
        "original_interpreter_sha256": hashlib.sha256(original.encode()).hexdigest(),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--proposal-source", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    arguments = parser.parse_args()
    record = prepare(
        arguments.root.resolve(),
        arguments.revision,
        arguments.proposal_source.resolve(),
    )
    record["instrumentation"] = {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in HERE.iterdir()
        if path.is_file()
    }
    arguments.manifest.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
