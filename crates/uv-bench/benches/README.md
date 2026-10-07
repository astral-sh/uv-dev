# Benchmarks

Run the commands below from the repository root.

## Git sources

`git_fetch` measures cold and cached fetches of pinned Python repositories and successive Django
releases through the local Git transport. The Rust fixtures download and verify the pinned histories
before measurement, then retain them in `target/bench-git` for subsequent runs.

```console
uv run --only-dev cargo codspeed build -m walltime --profile profiling -p uv-bench --bench git_fetch
uv run --only-dev cargo codspeed run -m walltime -p uv-bench --bench git_fetch git_fetch/
```

The `git_fetch_popular` group covers Requests and Transformers across four releases, with cold and
cached fetches for each combination of the two Git preview features. The `git_monorepo` group
resolves one or four Apache Airflow providers from a shared Git revision. These cases require Git
2.48 or newer. CI runs them in the existing walltime matrix.

```console
uv --no-config python install --install-dir .cache/bench-python 3.12.11
cargo build --locked --profile profiling --bin uv
uv run --only-dev cargo codspeed build -m walltime --profile profiling -p uv-bench --bench git_fetch
uv run --only-dev cargo codspeed run -m walltime -p uv-bench --bench git_fetch git_fetch_popular/
uv run --only-dev cargo codspeed run -m walltime -p uv-bench --bench git_fetch git_monorepo/
```

The larger Odoo history is opt-in. Run the `git_fetch_large/` filter with `UV_BENCH_LARGE_GIT=1`.
