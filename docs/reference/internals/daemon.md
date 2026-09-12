# Experimental daemon

Repeated project commands often rediscover the same workspace and deserialize the same `uv.lock`. A
batch interface can amortize that work for one command, but does not help independent invocations of
`uv export`, `uv sync`, `uv run`, `uv tree`, or tool commands. The daemon is an opt-in execution
mode for the ordinary CLI, not another package-management interface.

## User contract

`uv --daemon` enables the experimental mode for the current executable and starts a daemon for the
current Unix login session. Subsequent commands use it automatically. `uv --daemon <command>` also
enables the mode and executes that command. `uv --no-daemon <command>` bypasses it for one
invocation; standalone `uv --no-daemon` disables automatic routing and drains the current session's
daemon. `uv --daemon-status` prints its process, request, and cache counters as JSON.
`UV_DAEMON_DIR` overrides the private runtime directory for isolated deployments and testing.

The command's arguments, configuration precedence, environment, working directory, terminal, output,
exit code, and mutation semantics must remain the ordinary CLI's. The daemon is not allowed to
acquire permissions that the calling process lacks. Commands that cannot be delegated without
changing their process context execute locally. `--no-cache` also executes locally.

The draft supports Linux and macOS. It is not a promise that every command becomes faster: commands
without reusable work still pay the transport and worker costs. The first acceptance criterion is
that repeated lock-backed commands benefit without creating a separate export/sync/run API.

## Execution model

uv currently owns process-global state: environment flags, preview settings, logging, color,
working-directory state, signal handling, and native-library initialization. Its public binary entry
point is not reentrant. A long-running server must not call that entry point repeatedly with
changing environments.

The initial implementation is a single-threaded fork server. It starts from a fresh executable,
before any uv command initialization, and only handles local requests and parses source text. It
keeps parsed `Lock` values, their marker interner, and context-free project-manifest syntax in
memory. For each command it forks a new worker, which inherits those values through copy-on-write,
restores the caller's process context, and enters the normal uv implementation exactly once. The
parent never starts Tokio, Rayon, HTTP clients, Python, builds, or project hooks. Jemalloc
background threads are disabled in the parent, and the kernel-reported thread count is checked
before each fork.

The daemon is session-scoped because a Unix process can only join a process group in its own
session. The worker joins the caller's process group, so terminal-generated signals and job control
reach the same foreground group as they do with normal uv. The client passes actual stdin, stdout,
stderr, and an open working-directory descriptor with `SCM_RIGHTS`; output is not reconstructed from
text messages or a pseudo-terminal. The request also includes byte-preserving arguments and
environment, umask, resource limits, and inherited ignored/blocked signals. Commands with additional
inheritable file descriptors execute locally; the initial protocol does not transfer arbitrary
descriptor sets. The client forwards direct signals using the same interactive-SIGINT distinction as
uv's existing child runner.

This model deliberately does not retain HTTP clients, credentials, project configuration, workspace
discovery results, or mutable environments across commands. Those objects have different validity
and isolation rules. In particular, `WorkspaceCache` currently memoizes failed discoveries and
assumes a single operation's filesystem view; making it persistent without dependency tracking would
be incorrect.

## Lockfile validity

The shared cache sits at `Lock::from_toml`, so all ordinary callers of that parser benefit,
including project, script, and tool lockfiles. Workers still read the file through their normal
command path. They look up the exact bytes that were read, first using a digest and then confirming
byte equality. The working directory is also part of the key: deserialized local requirements
currently contain temporary absolute URLs. Each parse scopes its directory once, avoiding
per-requirement filesystem queries. The parent parses each warm request in its originating
directory, without initializing uv's invocation-specific working-directory cache. Cross-directory
reuse requires making the parsed representation location-independent first. A changed file therefore
cannot reuse its previous parsed value, even when its size and timestamps are unchanged. Missing
files and read errors keep their normal behavior. A command that already read a file uses that
immutable snapshot; a later command reads the new snapshot.

The parent caches only successfully parsed values under the parser's strict default policy. The
wheel-filename compatibility flag is part of cache eligibility: a permissively parsed value must
never satisfy a strict reader. Parse errors are not cached. A miss is parsed normally in the worker
and sent to the parent for independent parsing and reuse by later workers. `uv export`, `uv tree`,
`uv sync`, and `uv run` retain an `Arc<Lock>` snapshot for frozen reads instead of cloning the
package graph. Callers that need an owned `Lock` use the same parser and receive an independent
value.

The initial budget is 32 entries and 128 MiB of source text, with oldest-entry eviction. This bounds
the number and source size of retained values, not their full heap footprint. Before enabling the
feature by default, measure parsed-object and marker-interner memory, add an RSS/heap budget, and
consider restarting an idle parent after its interner or copy-on-write footprint grows too large.

File watchers are not a correctness requirement. A later watcher-backed cache may avoid repeated
reads, but a watcher event is only a dirty hint. It must track file identity, detect watcher
overflow and replacement, and revalidate content before publishing a new snapshot. Persisting
workspace discovery additionally requires tracking all consulted `pyproject.toml`, `uv.toml`, member
globs, missing paths, and configuration/environment inputs. No watcher optimization may silently
weaken the content-based validity contract.

## Project-manifest validity

`PyProjectTomlSource` retains the TOML parser's original value types, source text, and diagnostic
spans. The process cache uses the exact source bytes as its key. Every reader still reads the
manifest, then deserializes a fresh `PyProjectToml` in its own invocation. Environment expansion,
relative URLs, file-type checks, time-sensitive settings, and project validation are therefore not
reused across requests. Valid syntax may be retained even when typed validation fails; the next
reader evaluates that validation again. Syntax errors and filesystem read errors follow the normal
command path. The per-invocation `WorkspaceCache` and its mutation/invalidation rules are unchanged.

The syntax cache admits at most 16,384 entries and 32 MiB of source text, with a 256 KiB limit per
manifest and oldest-entry eviction. These are admission limits, not a bound on syntax-tree heap
allocations or the cost of forking a populated parent. Larger manifests use the ordinary parser.

Workers queue source observations without doing socket I/O in parser callbacks. The queue is bounded
to 8,192 sources and 8 MiB; transfer batches contain at most 256 sources and 1 MiB of text in a
separately bounded 2 MiB frame. Checkpoints after settings discovery, before an external command is
spawned, and at normal invocation completion make those sources available to later workers. The
final checkpoint closes the observer before draining it. Transfers are best effort, have a
one-second flush budget, and are not retried after an ambiguous acknowledgment. A parent accepts
batches only from its active worker PIDs and checks its batch deadline between source parses. It
never lowers project fields or starts an additional thread while warming this cache.

`cached_manifests` and `cached_manifest_source_bytes` describe retained syntax, not RSS.
`manifest_cache_hits` counts reported exact-source lookups, including reuse within one worker; it is
not exclusively a cross-invocation hit count. `manifest_cache_dropped` counts known source
observations omitted while queueing or warming the parent. It excludes oversized manifests that
bypass admission and observations lost with an interrupted worker or unacknowledged transfer.

## Protocol and lifecycle

The socket is inside an owner-only directory. Both peers verify the Unix socket credentials; a run
request also verifies that its claimed PID is the authenticated peer. The endpoint is keyed by
protocol generation, executable identity, and login session, so different installations or replaced
binaries do not accidentally exchange native state. The transport uses bounded, length-prefixed
MessagePack frames. It accepts no TCP connections and never logs request environments or lock text.

Startup uses an exclusive lock file. The holder may remove a stale, same-owner socket and bind the
replacement. Concurrent starters converge on that socket. A parent handles at most 64 workers and
exits after 15 idle minutes. Child exits wake the listener through a nonblocking `SIGCHLD` pipe;
notifications may be coalesced, so each wakeup reaps every exited worker. A graceful stop rejects
new work and waits for existing workers before removing its socket. Disabling the mode prevents
future automatic restarts; other session daemons retire when idle.

Submission has an explicit boundary. Before a request is submitted, an unavailable daemon can fall
back to normal uv. A `RunLocally` response also guarantees that no worker was started. After a
worker has been accepted, disconnects or daemon failure have an unknown outcome and must never cause
an automatic retry: the command could already have changed files, installed packages, published an
artifact, or started a user program. The surviving worker is not killed merely because the client
connection disappears.

Existing filesystem locks continue to arbitrate package-cache writes, lockfile updates, and
environment mutations. Parsed-lock reuse does not replace those locks or make concurrent mutations
transactional. Cold misses may temporarily duplicate parsing; avoiding that duplication must not
turn the parent into a global command-execution lock.

## Confinement and platform boundaries

A same-user socket is necessary but insufficient authorization to execute a command. A confined
client must not use an unconfined daemon as an execution service. The macOS implementation declines
delegation if either process is sandboxed. Linux declines
seccomp/no-new-privileges/capability-bearing contexts and requires matching credentials, groups,
namespaces, filesystem root, cgroup, and security label, CPU/memory affinity, and process priority.
If those properties cannot be inspected or the caller's hard resource limits cannot be reproduced,
the server requests local execution. These checks must remain fail-closed as the protocol evolves.
Container and sandbox support should use a daemon launched within the exact confinement domain or
retain foreground execution; comparing only UID, a session number, or a seccomp mode is not
sufficient.

Windows cannot use the copy-on-write fork model. The long-term cross-platform design needs a
request-scoped command engine with explicit environment/configuration, output, cancellation, and
process-launch interfaces. The daemon can then own immutable parsed state while the foreground
client owns terminal interaction and external-program execution. Alternatively, isolated workers can
use a versioned native lock representation over shared memory, but that representation must account
for process-local marker interning, schema evolution, and validation. Neither approach should expose
an unstable Rust object layout as a public protocol.

## Evaluation and next steps

Compare daemon-off, cold-daemon, and warm-daemon runs on the same executable and filesystem state.
Measure wall time, CPU time, parent/worker RSS, cache hit rate, and concurrent throughput for a real
large workspace, small projects, 27 independent exports, no-op sync/run, tree, and commands with no
lockfile. Compare against batch export without attributing batch's avoided CLI startups to lock
parsing alone.

Correctness coverage needs byte-for-byte output comparison, separate working directories and
environments, non-UTF-8 arguments, file replacement with unchanged timestamps, deletion, parse
failure, strict/permissive parser settings, concurrent commands, stdin/TTY behavior, signals and
exit codes, recursive uv calls, stale sockets, simultaneous startup, version mismatch, daemon crash,
confinement fallback, and graceful shutdown. The feature remains hidden until those contracts and
the single-threaded-parent audit are strong enough for a public experimental interface.
