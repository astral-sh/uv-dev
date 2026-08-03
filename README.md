# adjust-ulimit (0.12.0) makes every tcsh/csh subprocess ~100x slower

Issue: astral-sh/uv#20923
Classification: bug

## Summary

No prior tracker covers this specific regression. The closest history is the low-limit failure that motivated adjust-ulimit, its preview implementation, and its stabilization as the uv 0.12 default.

## Classification

This is a newly reported performance regression, not a duplicate: no existing issue or pull request tracks the tcsh/csh slowdown. Repository source confirms that uv raises the soft RLIMIT_NOFILE to the hard limit subject to a 1,048,576 cap, states that child processes inherit it, and warns that code iterating over every possible descriptor can time out. astral-sh/uv#20225 made that behavior default in uv 0.12, matching the reported onset. The reporter's tcsh implementation details and exact 100x measurement were not independently confirmed from this repository, but the source-backed behavior and reproducible version-specific degradation establish incorrect behavior appropriate for the bug label.

## Related

- https://github.com/astral-sh/uv/issues/16999 (closed issue): Bytecode compilation can fail with "too many open files" on default Ubuntu settings
  This is the motivating issue for automatic limit adjustment and was closed by astral-sh/uv#17464. It concerns the inverse symptom—uv exhausting a low limit during bytecode compilation—not subprocess slowdown, but establishes why uv began raising RLIMIT_NOFILE.
- https://github.com/astral-sh/uv/pull/17464 (merged pull request): Adjust the process ulimit to the maximum allowed on startup
  This introduced the exact limit-raising behavior behind the report. Its description acknowledged risks from increased limits and initially gated the behavior behind preview; its review explicitly noted that programs iterating over all possible file descriptors can time out. It did not report the tcsh/csh regression itself.
- https://github.com/astral-sh/uv/issues/20185 (closed issue): uv 0.12 preview stabilization tracking issue
  This is the canonical uv 0.12 stabilization discussion. Maintainers specifically considered whether adjust-ulimit had caused problems before deciding to stabilize it; astral-sh/uv#20225 closed the tracker.
- https://github.com/astral-sh/uv/pull/20225 (merged pull request): Stabilize preview features for uv 0.12
  This made adjust-ulimit the default for uv 0.12, matching the reported regression boundary. The PR raises the open-file limit at startup but contains no tcsh/csh-specific report or remedy.

## Reproduction

Status: `needs_more_information`

uv 0.12.1 raised a controlled RLIMIT_NOFILE from (1024, 65536) to (65536, 65536), confirming the behavior introduced by astral-sh/uv#20225. However, `uv run --no-project -- tcsh -f -c 'exit 0'` did not become slower with Ubuntu tcsh 6.24.10 or 6.21.00: raised-versus-capped medians were about 21–23 ms, and strace recorded 47 close calls at both limits. The runner’s hard limit prevented testing 524288, and its uv build is GNU rather than the reported musl build. Reproduction needs the exact container image/tag or Dockerfile, full `tcsh --version`, `readlink -f /bin/csh`, and kernel version. The existing test `crates/uv/tests/it/resource_limits.rs::adjust_open_file_limit` verifies limit inheritance but not tcsh performance.
