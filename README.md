# uv auth login: credentials file and directory permissions follow umask instead of being owner-only

Issue: astral-sh/uv#23202
Classification: bug

## Summary

The reporter describes `uv auth login` on Linux with uv 0.12.21 and the default plaintext backend. With umask `022`, the reported credential directory is `0755`, `credentials.toml` is `0644`, and its coordination lock is `0666`. Other local users can read the credential file when every ancestor permits traversal. The requested behavior is an owner-only directory (`0700`) and credential file (`0600`) from creation, restricted lock permissions, safe atomic replacement, and consideration of tightening existing stores.

The reported permissions were independently reproduced on Linux with the installed uv 0.13.7: umask `022` created a `0755` credential directory, a `0644` credential file, and a `0666` lock. A fresh store under umask `077` had directory/file modes `0700`/`0600`, but its lock remained `0666`. Writing to the permissive store under umask `077` did not tighten its modes. The lock was empty in both fresh-store cases; source inspection confirms that the shared locking helper explicitly overrides umask for that coordination file.

No duplicate of the credential-store permission bug was found. astral-sh/uv#16769, astral-sh/uv#17961, and astral-sh/uv#17646 concern general lock permissions; merged astral-sh/uv#16845 explains the current 0666 lock mode.

## Draft response

Reproduced with uv 0.13.7 on Linux using an isolated plaintext store and dummy input: umask 022 produces directory/file/lock modes 0755/0644/0666. A fresh store under umask 077 uses 0700/0600 for the directory and credential file, while the lock remains 0666. Rewriting the permissive store under umask 077 does not tighten existing permissions. The lock is an empty coordination file whose mode is explicitly set by the shared locking helper; astral-sh/uv#16769 discusses that separate policy. Owner-only credential contents and directory access remain the main concern.

## Classification

The runtime reproduction confirms that the default plaintext backend does not enforce owner-only permissions or tighten existing modes. The checked-out source creates the credential directory with create_dir_all and writes credentials with fs::write, consistent with those observations. With umask 022 and traversable ancestors, this can expose stored credentials to other local users. Documented plaintext storage does not justify that access. The 0666 lock mode is separately imposed by the shared locking helper. The open lock-permission issue and pull requests do not address owner-only credential contents or directories, so they are not duplicates of the main bug. No prior fix for this disclosure was found.

The primary correctness problem is access to private credential contents. The open request about general lock-file permissions covers only one part of this report, so it is insufficient grounds to centralize this issue there. Neither historical locking change established owner-only credential storage, and there is no evidence that a previous credential-permission fix has regressed.

## Reproduction

Outcome: **reproducible** for astral-sh/uv#23202.

The report specifies Linux, uv 0.12.21, the default plaintext backend without native authentication/keyring, and umask `022`. It expects a credential directory created as `0700`, a credential file created as `0600`, and a restricted lock. It also requests atomic writes and consideration of tightening existing stores. This is not presented as an upgrade regression.

Observed environment: Linux `7.0.0-1017-azure`, x86_64; installed `uv 0.13.7 (x86_64-unknown-linux-gnu)` from `/opt/hostedtoolcache/uv/0.13.7/x86_64/uv`. Python 3.12.3 was used only to orchestrate subprocesses and inspect filesystem metadata; `uv auth login` does not require a project or Python interpreter. The reported uv 0.12.21 was not separately tested.

The reproduction used a fresh temporary directory under `/tmp`, a clean child-process environment, an isolated home/config/cache/data directory, no inherited preview or credentials-directory override, and `--no-config --offline`. Only synthetic dummy tokens were supplied over stdin. No credential contents were inspected or printed, no service was contacted, and temporary stores were deleted afterward.

Equivalent minimal Bash commands using the installed executable:

```bash
uv_executable=$(command -v uv)
reproduction_directory=$(mktemp -d /tmp/uv-23202.XXXXXX)
mkdir -m 700 "$reproduction_directory/home"
(
    cd "$reproduction_directory"
    umask 022
    env -i PATH=/usr/bin:/bin \
        HOME="$reproduction_directory/home" \
        XDG_CONFIG_HOME="$reproduction_directory/config" \
        XDG_CACHE_HOME="$reproduction_directory/cache" \
        UV_CACHE_DIR="$reproduction_directory/cache/uv" \
        TMPDIR="$reproduction_directory" \
        XDG_DATA_HOME="$reproduction_directory/data-022" \
        "$uv_executable" --no-config --offline auth login \
        --token - https://example.invalid/simple <<<'dummy-token'
)
stat -c '%a %n' \
    "$reproduction_directory/data-022/uv/credentials" \
    "$reproduction_directory/data-022/uv/credentials/credentials.toml" \
    "$reproduction_directory/data-022/uv/credentials/credentials.toml.lock"
rm -rf -- "$reproduction_directory"
```

Each of the three explicitly invoked login commands exited `0` with empty stdout. The first two reported `Stored credentials for https://example.invalid/`; the rewrite used `https://second.example.invalid/simple` and reported `Stored credentials for https://second.example.invalid/`.

| Scenario | Credential directory | credentials.toml | credentials.toml.lock |
| --- | --- | --- | --- |
| Fresh store, umask `022` | `0755` | `0644` | `0666` |
| Separate fresh store, umask `077` | `0700` | `0600` | `0666` |
| Add a second dummy service to the first store under umask `077` | `0755` | `0644` | `0666` |

The umask `077` fresh-store variant changed only the umask and data directory. The rewrite variant reused the umask `022` store, changed the umask to `077`, and added a distinct dummy service. Both new lock files had size zero. These results distinguish credential directory/file modes, which follow umask on creation, from the lock mode, which overrides it. They also confirm that a stricter umask does not repair an existing permissive store.

The temporary root itself was `0700`; no access from another user was attempted. The observed `0644` mode grants group/other read permission subject to ancestor traversal and other filesystem controls. Atomicity and permissions at every instant during creation were not tested.

Existing coverage was read in `crates/uv/tests/it/auth.rs`: `login_text_store` checks successful password/token persistence and invalid inputs; `login_text_store_empty_file` checks login and token retrieval with an empty file via `UV_CREDENTIALS_DIR`; `login_text_store_comments_only_file` checks login with a comments-only file. Their `TestContext` setup isolates home/data/cache paths. None asserts Unix credential directory/file/lock modes or tightening on rewrite. Searches of the authentication source/tests and `crates/uv-client/tests/it/` found no matching permission regression test. The authentication integration module is unconditional in `crates/uv/tests/it/main.rs`; future mode assertions need a Unix gate. No repository tests were added or run, and no build was performed.

## Related

- astral-sh/uv#16769 (open issue): Add Setting To No Longer Force Lock Files to have 777 Permissions in Linux. Overlaps the lock-file permission concern: requests configurable permissions instead of overriding umask. Maintainers explain that these coordination files are empty and used across users or containers. It does not track readable plaintext credential contents or credential-directory permissions, so it is not a duplicate of the main bug.
- astral-sh/uv#17961 (open pull request): Add preview option to respect umask when creating lock files. Proposes a respect-umask preview feature for coordination locks and references astral-sh/uv#16769 and astral-sh/uv#17646. Its current diff explicitly keeps override_umask=true for credential-store locks and does not restrict credential-file or directory modes, so it does not fix the reported disclosure.
- astral-sh/uv#17646 (open pull request): Create lock files 644, only check read bit for locks, respect umask. Attempts to resolve astral-sh/uv#16769 by changing the generic lock helper. Maintainer comments identify compatibility problems with older uv versions and suggest preview gating. It overlaps the lock-file request but does not enforce owner-only permissions on stored credentials or their directory.
- astral-sh/uv#16845 (merged pull request): Use 0o666 permissions for flock files instead of 0o777. Merged on 2025-12-01, this changed the shared locking helper from 0777 to 0666 while retaining cross-user access. It explains the reported lock-file mode but changed neither credential-file nor credential-directory permissions; this report is not a regression of that fix.

## Supporting evidence

Source was inspected at checkout commit `c17e9952b317efd1b7a7c196dd34db262e488a7a`, dated 2026-11-10.

- [Backend selection](https://github.com/astral-sh/uv/blob/c17e9952b317efd1b7a7c196dd34db262e488a7a/crates/uv-auth/src/store.rs#L30-L49): `AuthBackend::from_settings` selects the text store unless native authentication is enabled.
- [Credential path and lock setup](https://github.com/astral-sh/uv/blob/c17e9952b317efd1b7a7c196dd34db262e488a7a/crates/uv-auth/src/store.rs#L243-L268): the store supports `UV_CREDENTIALS_DIR`, otherwise uses the credentials state bucket. Lock acquisition creates the parent with `fs::create_dir_all` before calling `LockedFile::acquire`.
- [Credential persistence](https://github.com/astral-sh/uv/blob/c17e9952b317efd1b7a7c196dd34db262e488a7a/crates/uv-auth/src/store.rs#L307-L335): `TextCredentialStore::write` serializes credentials, calls `fs::create_dir_all`, and then `fs::write`. It supplies no restrictive mode or permission adjustment, and has an atomic-write TODO. Rewriting an existing file does not tighten its permissions.
- [Login persistence path](https://github.com/astral-sh/uv/blob/c17e9952b317efd1b7a7c196dd34db262e488a7a/crates/uv/src/commands/auth/login.rs#L119-L127): the text backend inserts the supplied credentials and invokes that write method.
- [Unix coordination-lock creation](https://github.com/astral-sh/uv/blob/c17e9952b317efd1b7a7c196dd34db262e488a7a/crates/uv-fs/src/locked_file.rs#L298-L335): `DESIRED_MODE` is `0o666`; permissions are set after temporary-file creation specifically to override umask. Consequently, `umask 077` alone does not restrict a newly created coordination lock.
- [Authentication documentation](https://github.com/astral-sh/uv/blob/c17e9952b317efd1b7a7c196dd34db262e488a7a/docs/concepts/authentication/http.md#L28-L49): plaintext storage is documented, with native secret storage available in preview. This establishes the backend's intended format, rather than a reason to allow other local users access to its contents.
- In astral-sh/uv#16769, maintainer comments explain that coordination files are empty and uv does not read their contents. Later comments discuss validating tighter modes without breaking cross-user locking.
- Both astral-sh/uv#11328 and astral-sh/uv#16845 modified the shared filesystem locking implementation, not the plaintext credential writer. Their descriptions, comments, and reviews explain the multi-user coordination requirement.

The runtime observations in Reproduction independently confirm the numeric modes using the installed uv 0.13.7. Source inspection above describes the current checkout; it is supporting evidence, not a substitute for observing the installed executable. Only filesystem metadata and login status were inspected.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests with authenticated gh. Decomposed the report into credential-file readability, directory traversal permissions, lock-file modes, atomic writes, and tightening existing stores. Literal searches covered credentials.toml, credentials.toml.lock, TextCredentialStore, umask, 0600, 0700, and 0666; conceptual searches covered auth permissions, world-readable files, plaintext storage, owner-only access, chmod, and restricted credentials. Text searches were supplemented by a paginated scan of all 13,511 pull-request titles across states, with targeted inspection of relevant candidates. Fix-oriented searches covered closed umask reports, atomic credential writes, and linked permission changes. Inspected candidate bodies, comments, linked issues, and pull-request reviews. Ruled out astral-sh/uv#8810 and astral-sh/uv#10878 (authentication storage/keyring capabilities), astral-sh/uv#17867 (cached package hardlink permissions), and astral-sh/uv#18205 (system-wide credential configuration). astral-sh/uv#12665 and astral-sh/uv#11324 concern generic coordination locks, not credential contents. Inspected astral-sh/uv#17961 and astral-sh/uv#17646, including the former's credential-store diff; neither addresses owner-only credential contents. Also inspected astral-sh/uv#15588 (plaintext backend introduction, including review comments) and astral-sh/uv#15610 (credential-store concurrency locking), neither a prior permissions fix.

Additional comparisons:

- astral-sh/uv#12665 asks about the security implications of world-writable coordination files; it does not describe readable credential contents.
- astral-sh/uv#11324 was a uv 0.5.29 failure when one user tried to acquire an environment lock created by another. Its linked fix, astral-sh/uv#11328, broadened lock permissions.
- astral-sh/uv#8810 concerns secure authentication configuration and keyring integration; its comments also discuss native-auth failures. It does not track owner-only filesystem permissions.
- astral-sh/uv#10878 requested the login-and-save capability. Its plaintext-storage warning concerns encryption, without identifying this filesystem-permission problem.
- astral-sh/uv#17867 concerns package files hardlinked from a cache retaining an earlier umask. Its mechanism and expected behavior differ from credential storage.
- astral-sh/uv#18205 discusses system-wide credential configuration. A maintainer recommends `UV_CREDENTIALS_DIR`; this is relevant when considering compatibility for explicitly shared stores, but does not establish that default per-user storage should be readable by others.

The current diff in astral-sh/uv#17961 passes `true` for the new `override_umask` argument at the credential-store lock call. Its proposal therefore does not even remove the credential lock's explicit umask override. In astral-sh/uv#17646, maintainer comments explain that changing shared lock permissions can break access by older uv versions and suggest preview gating. Neither proposal hardens the credential file or directory.

The plaintext backend was introduced by merged astral-sh/uv#15588 on 2025-08-30. Merged astral-sh/uv#15610 added credential-store locking on 2025-09-02 to prevent concurrent writes from dropping credentials. These establish feature history, rather than an earlier fix for owner-only permissions.

## Suggested next steps

Restrict the credential directory before acquiring its lock, and create the credential file with owner-only permissions before writing any contents. Evaluate atomic replacement and safe handling of existing permissive stores. Keep changes to credential storage distinct from the shared lock policy, whose cross-user requirements are documented in the related discussions; explicitly configured shared credential directories also need consideration.

Regression coverage belongs with the existing authentication integration tests and should check Unix modes for new and existing stores without exposing credential contents. No implementation or tests were changed as part of this handoff.
