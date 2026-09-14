//! Source freshness keys over real, pinned release-tag histories.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::hint::black_box;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{is_codspeed_simulation, run_command};
use uv_cache_info::CacheInfo;

#[derive(serde::Deserialize)]
struct Fixture {
    name: String,
    commit: String,
    tags: BTreeMap<String, String>,
}

fn git(directory: &Path, config: &Path) -> Command {
    let mut command = Command::new("git");
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(name);
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", config)
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("-C")
        .arg(directory);
    command
}

fn update_refs(project: &Path, config: &Path, updates: &str) {
    if updates.is_empty() {
        return;
    }
    let mut child = git(project, config)
        .args(["update-ref", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Git ref update");
    child
        .stdin
        .take()
        .expect("Missing Git input stream")
        .write_all(updates.as_bytes())
        .expect("Failed to write Git ref updates");
    let output = child.wait_with_output().expect("Failed to update Git refs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn configure_keys(project: &Path, tags: bool) {
    let path = project.join("pyproject.toml");
    let mut metadata: toml::Table =
        toml::from_str(&fs_err::read_to_string(&path).expect("Failed to read project metadata"))
            .expect("Invalid project metadata");
    let uv = metadata
        .entry("tool")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .expect("Invalid tool table")
        .entry("uv")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .expect("Invalid uv table");
    uv.insert(
        "cache-keys".to_string(),
        toml::Value::Array(vec![toml::Value::Table(
            toml::toml! { git = { commit = true, tags = tags } },
        )]),
    );
    fs_err::write(
        path,
        toml::to_string_pretty(&metadata).expect("Invalid cache-key configuration"),
    )
    .expect("Failed to configure cache keys");
}

fn source_git_cache_keys(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../../scripts/benchmark/git-tags.json"))
            .expect("Invalid Git tag fixtures");
    let mut group = c.benchmark_group("source_git_cache_keys");
    for fixture in fixtures {
        for layout in ["loose", "packed"] {
            let directory = tempfile::tempdir().expect("Failed to create Git directory");
            let config = directory.path().join("gitconfig");
            fs_err::write(&config, "").expect("Failed to create Git configuration");
            let project = directory.path().join("project");
            let source =
                std::path::absolute(format!("../../.cache/bench-git-tags/{}.git", fixture.name))
                    .expect("Failed to locate Git fixture");
            run_command(
                git(directory.path(), &config)
                    .args(["clone", "--quiet", "--shared", "--no-checkout", "--no-tags"])
                    .arg(source)
                    .arg(&project),
            );
            run_command(git(&project, &config).args([
                "checkout",
                "--quiet",
                "--detach",
                &fixture.commit,
            ]));
            let output = git(&project, &config)
                .args(["for-each-ref", "--format=%(refname)", "refs/tags"])
                .output()
                .expect("Failed to list Git tags");
            assert!(output.status.success());
            let mut deletes = String::new();
            for reference in String::from_utf8(output.stdout)
                .expect("Invalid Git refs")
                .lines()
            {
                writeln!(&mut deletes, "delete {reference}").unwrap();
            }
            update_refs(&project, &config, &deletes);
            let mut creates = String::new();
            for (reference, oid) in &fixture.tags {
                writeln!(&mut creates, "create {reference} {oid}").unwrap();
            }
            update_refs(&project, &config, &creates);
            if layout == "packed" {
                run_command(git(&project, &config).args(["pack-refs", "--all"]));
            }
            for (scope, tags) in [("commit_only", false), ("commit_and_tags", true)] {
                configure_keys(&project, tags);
                let expected =
                    CacheInfo::from_directory(&project).expect("Failed to read Git cache keys");
                assert!(!expected.is_empty(), "Git cache key was not read");
                group.bench_function(
                    BenchmarkId::new(format!("{layout}/{scope}"), &fixture.name),
                    |b| {
                        b.iter(|| {
                            black_box(
                                CacheInfo::from_directory(black_box(&project))
                                    .expect("Failed to read Git cache keys"),
                            )
                        });
                    },
                );
                assert_eq!(
                    CacheInfo::from_directory(&project).expect("Failed to re-read Git cache keys"),
                    expected
                );
            }
        }
    }
    group.finish();
}

criterion_group! {
    name = git_keys;
    config = common::walltime_criterion();
    targets = source_git_cache_keys
}
criterion_main!(git_keys);
