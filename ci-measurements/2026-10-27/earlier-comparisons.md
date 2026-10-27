# Earlier uv action and cache comparisons

These measurements use the action revisions recorded below, before workspace cache keys included a
source fingerprint. Their timings apply to those earlier pins and measured cache states. The
refreshing action is measured separately in [the current report](README.md).

The fixed-source action-migration comparisons use full workspace caches in both variants for Linux
tests. They measure the action change, not the benefit of retaining workspace crates.

<details>
<summary>Fixed-source action-migration timings</summary>

Twenty paired runs per comparison, with fixed source and matching runner types. Mean times include
setup and post steps and exclude queues. Namespace Windows jobs use GitHub’s job start and end
timestamps because runner log timestamps can jump. No measured jobs saved a Rust cache. Table
parentheses show 90% confidence intervals for mean time saved. † marks jobs shortened to setup and
build steps; their post steps are included. Both variants restore exact Rust-cache hits in every
pair.

| Job / scope                                                                                          | Before → after (s) | Speedup |      Saved (s; 90% CI) |
| ---------------------------------------------------------------------------------------------------- | -----------------: | ------: | ---------------------: |
| [Linux tests](https://github.com/astral-sh/uv-dev/actions/runs/40118640607)                          |      172.9 → 158.4 |   1.09× |    14.5 (11.3 to 17.7) |
| [Linux x86_64 binary](https://github.com/astral-sh/uv-dev/actions/runs/40118423293)                  |        32.9 → 31.7 |   1.04× |      1.2 (-0.6 to 2.9) |
| [Linux ARM64 binary](https://github.com/astral-sh/uv-dev/actions/runs/40118642338)                   |        54.2 → 38.7 |   1.40× |    15.5 (12.6 to 18.3) |
| [Linux ARMv7 binary](https://github.com/astral-sh/uv-dev/actions/runs/40118644180)                   |        99.1 → 87.7 |   1.13× |    11.4 (-0.6 to 23.5) |
| [Linux musl binary](https://github.com/astral-sh/uv-dev/actions/runs/40118425396)                    |        49.1 → 43.2 |   1.14× |       5.9 (3.0 to 8.9) |
| [macOS ARM64 binary](https://github.com/astral-sh/uv-dev/actions/runs/40118427469)                   |        37.4 → 31.1 |   1.20× |       6.3 (5.3 to 7.4) |
| [macOS x86_64 binary](https://github.com/astral-sh/uv-dev/actions/runs/40118429809)                  |        38.7 → 32.8 |   1.18× |       5.9 (4.5 to 7.3) |
| [Windows x86_64 binary](https://github.com/astral-sh/uv-dev/actions/runs/40118803649)                |        88.0 → 64.1 |   1.37× |    23.9 (16.7 to 31.1) |
| [Windows ARM64 binary](https://github.com/astral-sh/uv-dev/actions/runs/40118805847)                 |      175.1 → 174.0 |   1.01× |      1.2 (-6.6 to 8.9) |
| [Android ARM64 binary](https://github.com/astral-sh/uv-dev/actions/runs/40118645988)                 |        48.0 → 32.3 |   1.49× |    15.7 (12.8 to 18.6) |
| [MSRV binary](https://github.com/astral-sh/uv-dev/actions/runs/40122521544)                          |        58.3 → 46.9 |   1.24× |     11.4 (9.1 to 13.8) |
| [FreeBSD binary](https://github.com/astral-sh/uv-dev/actions/runs/40122779035)                       |      138.4 → 137.6 |   1.01× |      0.8 (-5.3 to 6.9) |
| [macOS tests](https://github.com/astral-sh/uv-dev/actions/runs/40118807677)                          |      315.8 → 270.1 |   1.17× |    45.7 (41.7 to 49.6) |
| [Linux Clippy](https://github.com/astral-sh/uv-dev/actions/runs/40118651663)                         |        57.8 → 26.2 |   2.21× |    31.6 (29.2 to 34.0) |
| [Windows Clippy](https://github.com/astral-sh/uv-dev/actions/runs/40118653494)                       |        69.2 → 43.2 |   1.60× |    25.9 (23.2 to 28.6) |
| [Hawk](https://github.com/astral-sh/uv-dev/actions/runs/40122780961)                                 |      165.9 → 166.9 |   0.99× |   -1.0 (-15.1 to 13.0) |
| [Publish dry-run](https://github.com/astral-sh/uv-dev/actions/runs/40123110793)                      |      274.2 → 276.4 |   0.99× |     -2.1 (-8.2 to 3.9) |
| [Documentation](https://github.com/astral-sh/uv-dev/actions/runs/40118811750)                        |        83.3 → 69.5 |   1.20× |    13.8 (10.1 to 17.5) |
| [Generated files](https://github.com/astral-sh/uv-dev/actions/runs/40118879909)                      |        53.4 → 40.2 |   1.33× |    13.2 (10.5 to 16.0) |
| [Walltime benchmark build](https://github.com/astral-sh/uv-dev/actions/runs/40118936562)             |      296.3 → 278.1 |   1.07× |    18.1 (15.4 to 20.8) |
| [Simulated benchmark setup and build†](https://github.com/astral-sh/uv-dev/actions/runs/40119151358) |      424.5 → 310.7 |   1.37× | 113.8 (109.8 to 117.9) |

Measurements use source `6fe4f0f5`. The before variant uses each job’s pinned production action; the
after variant uses `ebba56fe` for dependency-only jobs and `4f88db2c` for workspace caches.

</details>
