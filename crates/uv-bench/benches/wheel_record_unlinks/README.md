# Wheel RECORD leaf-deletion comparison

This opt-in wall-time experiment compares the cost of removing pre-enumerated, independent
installed-wheel leaves. It does not select a production backend. The generated
`synthetic-manyfiles` fixture is a valid wheel with 10,007 installed `RECORD` leaves. The
`wheel-<full-wheel-filename>` fixture is a production copy-mode installation of the supplied,
SHA-256-verified wheel. Every operation receives a new byte-for-byte copy of that process's
installed template, including file modes and installed `RECORD` bytes/order. No trial uses
hardlinks to the input or template. Untimed metadata reports the full wheel filename, wheel
SHA-256, installed-manifest SHA-256, and leaf count. Installer-generated script paths and cache
metadata can change the installed manifest between processes; the wheel hash alone is not an
installed-bytes identity.

Set `WHEEL_PATH` to an absolute wheel path and `WHEEL_SHA256` to its independently recorded
SHA-256, then run:

```sh
mkdir -p "$HOME/code/tmp/uv-wheel-record-unlinks"
env UV_BENCH_UNLINK_SCRATCH="$HOME/code/tmp/uv-wheel-record-unlinks" \
  UV_BENCH_WHEEL_PATH="$WHEEL_PATH" \
  UV_BENCH_WHEEL_SHA256="$WHEEL_SHA256" \
  cargo bench --locked -p uv-bench --bench wheel_record_unlinks
```

All three `UV_BENCH_` settings are required together. With none set, or with
`CODSPEED_RUNNER_MODE=instrumentation`/`simulation`, the target skips cleanly. A partial tuple
is an error. The scratch directory must already exist, be private to the measurement, and
outlive every child process; do not put it beneath a `TempDir` guard that could recursively
delete quarantined children. External inputs are read-only. The target is not in the regular
CodSpeed wall-time target list.

## Timing boundaries

Every Criterion sample batch runs on a fresh submitting thread. Backend setup, a distinct
disposable warmup tree, and the final backend destructor are outside the timer. The helper sums
one measured call per newly recreated trial; reconstruction, source/installed hashing, source
and installed `RECORD` validation, filesystem safety checks, final-tree/error assertions, and
result/fixture destruction are outside those intervals.

| Benchmark ID | Included in each measured call |
| --- | --- |
| `wheel_record_unlinks/ordinary/<fixture>` | Ordered `fs_err::remove_file` calls and outcome-vector allocation. |
| `wheel_record_unlinks/workers-{1,4,16}/<fixture>` | Dispatch onto the reused bounded Rayon pool, all leaf calls, ordered collection, and outcome-vector allocation. |
| `wheel_record_unlinks/io-uring-{1,16,64,256}/<fixture>` | Relative-name checks and owned `CString` preparation, root-directory FD acquisition/close, bounded plain `UNLINKAT` submission/completion, ordered outcome allocation, and request-storage cleanup. |
| `wheel_uninstall_wheel/production/<fixture>` | The complete production `uninstall_wheel` call: installed `RECORD` parsing, scheme checks, serial removal/fallback behavior, and directory/`__pycache__` cleanup. |

These initial leaf rows reuse their pool or ring. They exclude backend construction/drop,
submitting-thread creation/teardown, and the task's first io-wq initialization. A later
setup-inclusive confirmation must name fresh-pool/fresh-ring API costs separately and compare
the selected controls on fresh trials. Rayon pool drop requests worker termination but does not
by itself join every worker OS thread; neither API construction/work/drop nor these reused rows
are fresh-process costs.

The Linux backend is compiled for `x86_64`, `aarch64`, `riscv64`, `loongarch64`, and `powerpc64`.
It requires runtime `UNLINKAT` opcode support and a successful disposable-file unlink. Effective
bounded/unbounded io-wq limits are reported when the kernel supports the registration and are
checked on each isolated sample thread. An unavailable backend is omitted, never relabeled
ordinary-I/O time. There is no forced-async, SQPOLL, worker-cap, or ordinary fallback mode.

## Result contract

The leaf API attempts **all** independent paths and returns one ordered result for each. The
untimed oracle compares successful counts and the exact remaining tree, including unrelated
sentinels. Error tests compare the ring to raw `std::fs::remove_file` errno/kind and the
contextual ordinary/Rayon controls by error kind. In particular, flags-zero `UNLINKAT` removes
a leaf symlink itself and does not implement recursive directory removal.

This is not production partial-failure equivalence: production can stop at an earlier error,
ignore a missing leaf, or recursively remove a directory, while independent workers can already
have removed later leaves. A production proposal needs an explicit failure policy and a
representative setup-inclusive/full-uninstall improvement over the best portable control.

An unrecoverable completion-control error retires the ring. If every submitted CQE cannot be
accounted for, the driver retains its bounded pending names, root FD, and fixture lease, marks
that unique root quarantined, and aborts the measurement. The pathname must not be removed or
recreated while those operations may still be running.
