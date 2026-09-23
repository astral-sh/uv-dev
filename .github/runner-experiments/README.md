# Namespace temporary filesystem comparison

This manually dispatched harness compares the default temporary directory with
`TMPDIR=/tmp/uv-tests` on a dedicated 16-GiB `tmpfs` mount. Both configurations use
the same tuned 32-vCPU runner and 40 nextest workers. The default directory may
already be `tmpfs`; filesystem inventories determine what was actually measured.
The dedicated mount exists but is unused by default-directory passes, so this
does not measure the cost of setting up the mount.

An excluded sample-zero pilot saves the Rust cache. After verifying the pilot,
dispatch one fixed cohort of samples 1 through 12 with cache saves disabled.
Concurrency is capped at five jobs. Each job runs an excluded default-directory
warmup and four measured suites. Odd samples use default/dedicated/dedicated/default;
even samples use dedicated/default/default/dedicated. This balances the measured
periods and allows same-configuration repeats within each job.

Require all 5,159 passing identities and four skips in each suite, and no measured
recompilation. Preserve failures and incomplete samples without replacement or
adaptive extension. Average each job's two measurements per configuration, then
calculate a ratio of arithmetic means and paired seconds saved. Use two-sided
90% paired bootstrap intervals over jobs, 50,000 resamples, and seed 20260923.
Treat ratios from 0.95 to 1.05 as the prespecified practical-equivalence band;
an inconclusive interval is not evidence of equivalence.

Boundary inventories and per-pass logs retain CPU work, memory and I/O counters,
mount options, test identities, and individual test durations. There is no live
profiler or security-mitigation change. Repeated suites measure warmed execution,
not first-suite production latency; five-suite job duration is not a normal CI
job timing. Runner names do not establish distinct physical hosts.
