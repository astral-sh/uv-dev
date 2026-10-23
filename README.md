# Intermittent silent exit with an error code - related to transient aws-lc dependency

Issue: astral-sh/uv#22830

Classification: bug

## Summary

The reporter observes intermittent silent termination with Windows exit code `0xC0000409` (`-1073740791`) during HTTPS operations in uv 0.13.4, also reported on 0.11.0 and 0.11.33. The reported stack reaches `abort()` from AWS-LC's `tree_jitter_initialize_once`, with `jent_entropy_collector_alloc` returning NULL. The report names `uv sync`, `uv tool run --with`, `uv build`, and `uv pip compile` as affected commands.

The closest upstream discussion is aws/aws-lc-rs#1072. The proposed jitter recovery change in aws/aws-lc#3631 was open at the previous inspection on 2026-10-22. No independent Windows reproduction or validation of that patch has been performed here.

## Current investigation

In the supplied discussion, maintainer woodruffw requested additional Windows/VM setup details, including the hypervisor, on 2026-10-23 at 14:43 UTC (comment ID 6367537627 on astral-sh/uv#22830). The maintainer regarded the diagnosis as plausible at a high level, while raising a concern about insufficient entropy in early boot environments. This is a request for clarification and an investigation concern, not confirmation of the precise root cause or approval of an entropy-source change.

The issue body already identifies QEMU/KVM and the environment below. The supplied discussion contains no reporter response to the request. It does not establish the guest's uptime at failure or whether this reproduction depends on early boot. Clarifying those points and any additional VM configuration relevant to entropy availability would help assess the maintainer's concern. Earlier comments in aws/aws-lc-rs#1072 describe failures after boot as well; that evidence does not establish the boot timing of this reporter's runs.

## Reported environment and reproduction

- Windows 11 Pro 10.0.26200, x86-64, QEMU/KVM guest identified as `Standard PC (Q35 + ICH9, 2009)`, with four vCPUs on an AMD Ryzen 5 3600 host and a QEMU virtual disk.
- uv runs as SYSTEM through a GitLab Runner shell executor. Defender real-time protection is enabled, with the working directory and uv cache excluded. Python is 3.12.15.
- The attached PowerShell reproduction starts from an empty directory, sets isolated cache and managed-Python directories under `uv-abort-repro`, writes an `iniconfig` requirement, and starts one file-churning process per CPU.
- It repeatedly launches fresh `uv pip compile` processes with the requirement file, managed Python, `--refresh`, and `--no-header`, collecting the exit code and redirecting stdout/stderr. The full script remains in the issue body; it has not been executed here.
- Under disk load, the reporter observed 16 failures in 1,000 official uv 0.13.4 runs, compared with none in 1,000 runs of a build made with `AWS_LC_SYS_NO_JITTER_ENTROPY=1`. A comparable custom build with jitter enabled also failed. Offline and unloaded controls did not fail in the reported samples.

The reporter reconstructed the release-binary stack and reports a NULL jitter collector in five captured failures. Some intermediate frames were inferred. The explanation that VM timing noise causes jitter health-test failures remains a hypothesis; neither the maintainer comment nor these observations establish an OS entropy shortage.

## Classification

Intermittent termination without diagnostics during HTTPS operations is incorrect behavior, so the classification remains bug. The reported stack and controlled rebuild results align with an existing upstream Windows jitter-initialization abort. Exact attribution and the reason collector initialization fails remain unverified here.

The related uv symbol-publishing request concerns release artifacts, while the closed ARM crash report concerns hardware RNDR failures. Neither establishes that this Windows collector-initialization failure was already fixed and has regressed. The maintainer's request for setup details does not change the classification.

## Related

States below were checked during the earlier investigation on 2026-10-22, not refreshed for this comment update.

- aws/aws-lc-rs#1072 — Application is occasionally aborted when runs as a Windows service during boot (open issue). The closest upstream match: comments include uv crashing on Windows VMware VMs through `RAND_bytes` and `tree_jitter_initialize_once`, and reports of failures after boot. An upstream contributor documents the build-time jitter opt-out.
- aws/aws-lc#3631 — Recover from CPU Jitter failures in non-FIPS builds (open pull request). The inspected diff falls back to OS entropy after failed jitter collector initialization or entropy reads in non-FIPS builds. Its applicability to this reproduction still needs validation.
- aws/aws-lc#3604 — CPU jitter seeding aborts the process on RK3399 (Cortex-A72, 24 MHz arch timer): ~12% of process starts (open issue). Motivates the recovery proposal, but involves ARM hardware and awscrt root seeding rather than uv's reported Windows collector-allocation failure.
- astral-sh/uv#20632 — Publish debugging symbols for official releases (open issue). Originated from intermittent Windows VM crashes linked to aws/aws-lc-rs#1072. Its requested work is publishing symbols, not fixing the crash.
- astral-sh/uv#21743 — Intermittent SIGABRT on aarch64 (GCP Axion / AWS Graviton) during package downloads - AWS-LC RNDR no-retry (closed issue). Similar silent TLS-time termination, but a different hardware entropy path. Its upstream chain leads through aws/aws-lc#3453 to the bounded RNDR retry fix aws/aws-lc#3475, merged on 2026-09-15; that fix does not address Windows jitter collector initialization.
- astral-sh/uv#18550 — Upgrade reqwest to 0.13 (merged pull request). Replaced ring with aws-lc, released in uv 0.11.0 on 2026-03-23. Establishes the reported dependency-change timing, not a crash fix.

## Supporting evidence and workaround limits

The previously inspected checkout pins aws-lc-rs 1.18.0, aws-lc-sys 0.44.0, and rustls 0.23.45 in `Cargo.lock`. `Cargo.toml` enables rustls's `aws_lc_rs` provider. The release workflow explicitly builds the pinned AWS-LC dependency rather than a system installation. The uv 0.11.0 changelog documents replacing ring with aws-lc.

`AWS_LC_SYS_NO_JITTER_ENTROPY=1` is an upstream-documented build-time workaround. Setting it when launching an existing uv executable does not change that binary. The reporter's comparison supports investigating this path, but does not establish the suitability of an entropy-source change for every environment. The maintainer has not endorsed a uv build change in the supplied discussion.

Useful next work is to clarify the VM setup and boot timing, then validate a candidate uv build incorporating the upstream recovery change against the reported workload, including collector-allocation failure. The reporter has offered to help verify a candidate build. Dependency uptake through aws-lc-sys remains necessary before the inspected upstream proposal can be treated as an official uv fix.

## Search coverage and limits

The earlier investigation used authenticated GitHub searches across open and closed uv issues and all PR states. Searches separated silent exits, Windows/intermittent crashes, command names, VMware, and exact exit codes from proposed causes such as jitter, entropy, AWS-LC, RNDR, and exact collector identifiers. It also inspected dependency migration and fix history, candidate comments, upstream chains, and 500 recent PR titles. Initial REST rate limiting was handled using GraphQL. PR keyword searches returned no matches despite the directly retrieved migration PR, so they do not establish exhaustive absence of a relevant PR.

Inspected alternatives excluded as different mechanisms included astral-sh/uv#12612 (SecuPrint injection), astral-sh/uv#17898 (reported Rust panic), astral-sh/uv#18735 (download timeouts), and astral-sh/uv#18890 (certificate validation).

This update incorporates the supplied issue discussion through the maintainer's 2026-10-23 comment. No reproduction, build, or fresh upstream status check was performed for this update.
