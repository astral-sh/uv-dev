# Intermittent silent exit with an error code - related to transient aws-lc dependency

Issue: astral-sh/uv#22830

Classification: bug

## Summary

The reporter observes intermittent silent termination with Windows exit code `0xC0000409` (`-1073740791`) during HTTPS operations in uv 0.13.4, also reported on 0.11.0 and 0.11.33. The reported stack reaches `abort()` from AWS-LC's `tree_jitter_initialize_once`, with `jent_entropy_collector_alloc` returning NULL. The report names `uv sync`, `uv tool run --with`, `uv build`, and `uv pip compile` as affected commands.

The closest upstream discussion is aws/aws-lc-rs#1072. The proposed jitter recovery change in aws/aws-lc#3631 was open at the previous inspection on 2026-10-22. No independent Windows reproduction or validation of that patch has been performed here.

## Current investigation

In the supplied discussion, maintainer woodruffw requested additional Windows/VM setup details, including the hypervisor, on 2026-10-23 at 14:43 UTC (comment ID 6367537627 on astral-sh/uv#22830). The maintainer regarded the diagnosis as plausible at a high level, while raising a concern about insufficient entropy in early boot environments. This is a request for clarification and an investigation concern, not confirmation of the precise root cause or approval of an entropy-source change.

The reporter's corrected response in comment ID 6387072669 on astral-sh/uv#22830 identifies the affected runner as a long-lived Windows VM using GitLab Runner's shell executor, reused across jobs without restarting between them. Multiple runner types could pick up jobs; the reporter says the failures occurred on this reused VM and were not observed on runners that created a fresh VM for each job. The reporter explicitly marked the earlier fresh-VM description as incorrect and called attention to the correction in comment ID 6387699059 on 2026-10-24 at 18:05 UTC.

The corrected account does not support treating fresh boot as the observed trigger. It does not establish an entropy shortage or rule out every timing-related mechanism. Earlier comments in aws/aws-lc-rs#1072 also describe failures after boot, but those remain separate observations. The difference between runner types is not a controlled comparison, so VM reuse alone is not established as the cause.

The reporter attributes the corrected environment summary to AI and has not independently verified the host configuration here. RAM and firmware values appeared only in the explicitly retracted summary and are not established for the affected runner. Exact host QEMU/KVM versions remain unspecified.

## Reported environment and reproduction

- The corrected account describes a long-lived Windows 11 Pro 26200 guest using libvirt/QEMU-KVM, Q35, and four vCPUs. The original report specifies x86-64 and a QEMU virtual disk.
- The guest reportedly runs on one AMD Ryzen 5 3600 host with six physical cores / twelve threads and Linux 6.17, shared with other VMs. Exact hypervisor versions are unspecified.
- GitLab Runner's shell executor runs as SYSTEM within the affected VM, which keeps running jobs without restarting between them. This corrects the retracted custom-executor/fresh-VM attribution and agrees with the shell-executor description in the original issue body.
- The corrected summary reports that 36 of approximately 350 CI jobs over a week encountered at least one uv exit with `0xC0000409` (about one job in ten), with more failures when many jobs ran concurrently. This is a job-level observation, distinct from the per-invocation reproduction measurements below; it does not establish a causal concurrency mechanism.
- The reporter did not observe the issue in CI jobs handled by runners that supplied a fresh VM for each job. No matched workload comparison or sample count for those runners is provided.
- The original writeup reports Defender real-time protection enabled with the working directory and uv cache excluded, and Python 3.12.15.
- The attached PowerShell reproduction starts from an empty directory, sets isolated cache and managed-Python directories under `uv-abort-repro`, writes an `iniconfig` requirement, and starts one file-churning process per CPU.
- It repeatedly launches fresh `uv pip compile` processes with the requirement file, managed Python, `--refresh`, and `--no-header`, collecting the exit code and redirecting stdout/stderr. The full script remains in the issue body; it has not been executed here.
- Under disk load, the reporter observed 16 failures in 1,000 official uv 0.13.4 runs, compared with none in 1,000 runs of a build made with `AWS_LC_SYS_NO_JITTER_ENTROPY=1`. A comparable custom build with jitter enabled also failed. Offline and unloaded controls did not fail in the reported samples.

The reporter reconstructed the release-binary stack and reports a NULL jitter collector in five captured failures. Some intermediate frames were inferred. The explanation that VM timing noise causes jitter health-test failures remains a hypothesis; neither the maintainer comment nor these observations establish an OS entropy shortage.

## Additional VM report and affinity workaround

Smallgiil's 2026-11-04 comment (ID 6631980054 on astral-sh/uv#22830) reports similar failures with uv 0.13.7 on Windows KVM VMs with eight vCPUs on a Xeon Gold 6248R host. The attached report describes silent `0xC0000409` exits 1–3 seconds after process start during the first TLS handshake, only for commands opening network connections. Rerunning succeeds; the contributor says failures occur almost never on physical machines, without providing physical-machine sample counts.

The contributor's AI-assisted report states that 32 crash dumps showed `__fastfail(7)` in `tree_jitter_initialize_once` after a NULL return from `jent_entropy_collector_alloc`. This is consistent with the original reported failure path, but the dumps were not supplied or independently checked here. No maintainer determination that both reports share an underlying cause appears in the supplied discussion.

The attachment reports these process-level measurements:

| Concurrent uv processes | Affinity pinned | CPU busy-loop processes | Runs | Crashes |
| --- | --- | --- | --- | --- |
| 1 | No | 0 | 300 | 0 |
| 1 | No | 8 | 200 | 26 |
| 8 | No | 0 | 400 | 38 |
| 8 | Yes | 0 | 400 | 0 |
| 8 | Yes | 8 | 400 | 0 |

Pinning each concurrent uv process to a separate vCPU is therefore a reported runtime workaround, with no crashes observed in these finite pinned samples. The report gives the Windows Command Prompt example `start "" /b /wait /affinity 1 uv.exe ...` to restrict one uv process to the first logical CPU. This limits that process to a single CPU and has not been validated here.

The report attributes the failures to thread migration across vCPUs with unsynchronized TSC readings during jitter startup tests. Affinity results support investigating scheduling and timing, but do not independently establish TSC skew, migration during the failing test, or the precise failed health check. CPU-only contention reproduced this contributor's failure, whereas the original reporter did not reproduce with CPU-only load; preserve that environment-dependent difference when comparing results.

### Attached reproduction details

The three attachments were read as text, without executing their contents:

- Report: https://github.com/user-attachments/files/34374237/README.md
- Unpinned script: https://github.com/user-attachments/files/34374252/jitter-repro.ps1.txt
- Pinned script: https://github.com/user-attachments/files/34374253/jitter-pinned.ps1.txt

Both scripts write `req.txt` containing `six` beside the script and invoke `uv pip compile` with `--no-cache`, `--python 3.11`, `--index-url https://pypi.org/simple`, and `-q` by default. The unpinned script accepts `-Runs`, `-Parallel`, and `-Burners`, launches batches of uv processes, saves failed-command output under `logs`, and prints an exit-code histogram. The pinned script accepts `-RunsPerCpu`, `-Cpus`, and `-Burners`; each PowerShell worker sets its own `ProcessorAffinity` before launching child uv processes, which inherit that affinity. It creates and removes `pinned-out` beside the script. Both can start CPU busy-loop processes.

The attached examples use 400 unpinned runs with eight concurrent processes, and 50 runs per CPU across eight pinned workers with eight busy-loop processes. The unpinned script's aggregate crash counter includes all exception exits at or above `0xC0000000`, while the pinned script counts only `0xC0000409`; compare the unpinned exit-code histogram when validating equivalent outcomes. Neither script nor the affinity command was run here.

## Classification

Intermittent termination without diagnostics during HTTPS operations is incorrect behavior, so the classification remains bug. The reported stack and controlled rebuild results align with an existing upstream Windows jitter-initialization abort. Exact attribution and the reason collector initialization fails remain unverified here.

The related uv symbol-publishing request concerns release artifacts, while the closed ARM crash report concerns hardware RNDR failures. Neither establishes that this Windows collector-initialization failure was already fixed and has regressed. The clarified VM lifecycle does not change the classification or establish the proposed entropy-shortage mechanism.

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

Useful next work is to characterize the affected long-lived shell-runner VM under concurrent jobs, recording host/guest load, guest uptime, and exact hypervisor versions. A matched workload comparison with runners using fresh VMs could help separate lifecycle effects from other configuration differences; the current observations do not isolate those factors. The additional eight-vCPU report supplies an affinity comparison worth reproducing with matched workloads and exit-code accounting; scheduling/TSC instrumentation would be needed to test its proposed mechanism. A candidate uv build incorporating the upstream recovery change also needs validation against the reported workload, including collector-allocation failure. The reporter has offered to help verify a candidate build. Dependency uptake through aws-lc-sys remains necessary before the inspected upstream proposal can be treated as an official uv fix.

## Search coverage and limits

The earlier investigation used authenticated GitHub searches across open and closed uv issues and all PR states. Searches separated silent exits, Windows/intermittent crashes, command names, VMware, and exact exit codes from proposed causes such as jitter, entropy, AWS-LC, RNDR, and exact collector identifiers. It also inspected dependency migration and fix history, candidate comments, upstream chains, and 500 recent PR titles. Initial REST rate limiting was handled using GraphQL. PR keyword searches returned no matches despite the directly retrieved migration PR, so they do not establish exhaustive absence of a relevant PR.

Inspected alternatives excluded as different mechanisms included astral-sh/uv#12612 (SecuPrint injection), astral-sh/uv#17898 (reported Rust panic), astral-sh/uv#18735 (download timeouts), and astral-sh/uv#18890 (certificate validation).

This update incorporates the supplied discussion through Smallgiil's 2026-11-04 report and a read-only review of its three attachments. No reproduction, build, or fresh upstream status check was performed for this update.
