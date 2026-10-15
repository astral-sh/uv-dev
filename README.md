# macOS release binaries: use a stable code-signing Identifier for uv and uvx

Issue: astral-sh/uv#22627

Classification: bug

## Summary

The changing code-signing identifiers are independently reproduced in the official macOS ARM64 release archives for uv 0.12.12, 0.12.24, and 0.13.1. All six identifiers match the report exactly. Each executable's embedded designated requirement includes its release-specific identifier, while the Team ID remains `2DC432GLL2`. The ARM64 PyPI wheels for 0.12.12 and 0.13.1 contain byte-identical uv and uvx executables to their corresponding archives, confirming the same behavior in those wheels.

The reporter expects a stable, distinct identity for each executable so identity-based rules survive upgrades. They report repeated Little Snitch approval prompts and incompatibility with allowlists that pin designated requirements. Those application-level effects were not exercised: artifact inspection ran on Linux, without macOS, Little Snitch, or an MDM/PPPC policy. The observed metadata establishes identifier changes, not whether every security product or policy necessarily prompts.

## Classification

Retain the bug classification as a release identity compatibility problem following the introduction of Developer ID signing. This is distinct from the completed request to introduce signing in astral-sh/uv#14870. No last-known-good signed release with stable identifiers is supplied or established; the earliest signed release cited by the report already has hash-suffixed identifiers. This reproduction therefore does not establish a regression introduced by a particular later upgrade.

The requested stable identifier naming contract remains a maintainer decision. Reproducing changing identifiers alone does not establish a signature-validity defect, and no signature-validity failure was observed or tested here. The source's omission of an explicit identifier is consistent with the report's inheritance explanation, but the linker-to-signer inheritance mechanism was not independently reproduced.

## Related

- astral-sh/uv#14870 — Sign published executables for macOS (closed issue). Maintainers confirmed that Developer ID signing shipped in 0.12.12 for official archives and PyPI wheels. The original request included organizational allowlists and repeated approvals after upgrades; this report concerns identity stability in the resulting signed executables. Maintainers also clarified that Homebrew builds its own binaries.
- astral-sh/uv#21754 — Use shared release signing actions (merged 2026-10-02). Moved signing, notarization, assembly, and verification into astral-sh/github-actions, following astral-sh/github-actions#3. The currently pinned shared signer does not pass `--binary-identifier`. The refactor does not establish identifier stability.
- astral-sh/uv#21526 — Sign release binaries from the wheel inventory (merged 2026-09-07). Signs the executable inventory extracted from wheels and verifies expected executable sets in wheels and GitHub archives. This connects the two distribution formats to the same signing pipeline.
- astral-sh/uv#22246 — Use 16 KiB code-signature pages for ARM64 macOS releases (merged 2026-10-06), following astral-sh/github-actions#8. Changes signature-verification performance, not program identity. Both this change and the shared-action refactor postdate the earliest affected release reported here.

## Reproduction

**Outcome: reproducible.** Independently downloaded public release artifacts contain exactly the changing identifiers reported. Their embedded designated requirements also contain the corresponding release-specific identifiers.

### Environment and scope

- Inspection host: Linux x86_64, kernel `6.17.0-1022-azure`; Python 3.12.3.
- Installed executable on `PATH`: `/opt/hostedtoolcache/uv/0.13.0/x86_64/uv`; `uv --version` reports `uv 0.13.0 (x86_64-unknown-linux-gnu)`.
- Inspected target: `aarch64-apple-darwin`, releases 0.12.12, 0.12.24, and 0.13.1. Release dates are 2026-09-09, 2026-10-08, and 2026-10-14 respectively.
- Additional artifacts: `uv-0.12.12-py3-none-macosx_11_0_arm64.whl` and `uv-0.13.1-py3-none-macosx_11_0_arm64.whl` from PyPI.
- The report used `codesign -dv` on macOS on 2026-10-15; it does not give the macOS version. Python version, project configuration, dependency groups, and resolver settings are not inputs to this artifact-metadata behavior.
- All reproduction files are under `/tmp/uv-22627-repro.WSDVpw`. No packages were installed, release binaries executed, self-updates performed, or existing user state changed. No build was required.

### Commands and inspection method

Downloaded the three named official archives into the temporary directory:

```sh
curl --fail --location --silent --show-error --output 0.12.12.tar.gz https://github.com/astral-sh/uv/releases/download/0.12.12/uv-aarch64-apple-darwin.tar.gz
curl --fail --location --silent --show-error --output 0.12.24.tar.gz https://github.com/astral-sh/uv/releases/download/0.12.24/uv-aarch64-apple-darwin.tar.gz
curl --fail --location --silent --show-error --output 0.13.1.tar.gz https://github.com/astral-sh/uv/releases/download/0.13.1/uv-aarch64-apple-darwin.tar.gz
python3 inspect-signatures.py 0.12.12.tar.gz 0.12.24.tar.gz 0.13.1.tar.gz
PYTHONDONTWRITEBYTECODE=1 python3 inspect-wheels.py
```

The temporary standard-library scripts are retained in `/tmp/uv-22627-repro.WSDVpw`, with complete results in `archive-signatures.jsonl` and `wheel-signatures.jsonl`. The archive reader opens only the named uv and uvx regular-file members, locates the Mach-O `LC_CODE_SIGNATURE` load command, and decodes its signature superblob. It reads Identifier and TeamIdentifier from the CodeDirectory offsets and parses the designated-requirement expression from requirements slot 3, checking that the whole expression was consumed. This measures embedded metadata directly rather than inferring behavior from source or searching arbitrary strings in the executables. The wheel reader selects the ARM64 macOS wheel using the version's public PyPI JSON metadata and inspects the two executable members with the same parser.

All three archive SHA-256 values match their published `.tar.gz.sha256` sidecars. Both wheel hashes match PyPI metadata. The inspected archives have these hashes:

| Release | Archive SHA-256 |
| --- | --- |
| 0.12.12 | `46740540b63fdee9a6cb2e19baf3f1f475b850c440a33e63455087a6871263f1` |
| 0.12.24 | `0c4346de7abdb49495b393b9ec809fe387aa43e586be20fecb972216c1e71732` |
| 0.13.1 | `cfe454573412ff7380d15fc7dae5997ab53ffce72fce0a78e91ddbc84001acd2` |

For a native macOS reproduction, use a fresh temporary directory, download the desired archive using the corresponding command above, and inspect both executables. For example, the following commands are provided for native follow-up and were **not** run on this Linux host:

```sh
tar -xzf 0.13.1.tar.gz
codesign --display --verbose=4 uv-aarch64-apple-darwin/uv
codesign --display --requirements - uv-aarch64-apple-darwin/uv
codesign --display --verbose=4 uv-aarch64-apple-darwin/uvx
codesign --display --requirements - uv-aarch64-apple-darwin/uvx
```

Repeat in separate temporary directories for 0.12.12 and 0.12.24 to compare Identifier, TeamIdentifier, and designated requirements.

### Observed results

| Release | uv Identifier | uvx Identifier | TeamIdentifier for both |
| --- | --- | --- | --- |
| 0.12.12 | `uv-924a20a6cc8daa66` | `uvx-52a8c5d340e3db83` | `2DC432GLL2` |
| 0.12.24 | `uv-7d60a5a1146de3cd` | `uvx-462788ef10328e23` | `2DC432GLL2` |
| 0.13.1 | `uv-12607afb3fd970b7` | `uvx-1704e7899e715f4e` | `2DC432GLL2` |

Every designated requirement is a conjunction containing that executable's Identifier, an Apple generic anchor, certificate-extension conditions, and a leaf certificate `subject.OU` equality condition for `2DC432GLL2`. Thus, the exact identifier condition embedded in one inspected release differs from the condition embedded in the others, despite a stable Team ID.

For both 0.12.12 and 0.13.1, uv and uvx executable SHA-256 values are identical between the archive and wheel. Those wheel binaries therefore have the same identifiers and designated requirements shown above. The 0.12.24 wheel and Intel macOS artifacts were not sampled.

### Existing coverage and limits

Searches of `crates/uv/tests/` and `crates/uv-client/tests/it/` for code signing, designated requirements, Team IDs, binary identifiers, and Little Snitch found no test covering release identifier stability.

The relevant existing verification is in the release pipeline:

- `.github/workflows/sign-release-binaries.yml` invokes the pinned shared `verify-release` action for assembled archives and wheels.
- At shared-action commit `b61c7106c7746375df8005eab240be92e4e450b2`, `release-artifacts/verify.py::verify` checks the executable inventories and dispatches native verification for both formats. `release-artifacts/verify-release-binaries-macos.py::verify_binaries` checks exact packaged bytes, `codesign --verify --strict -R '=anchor apple generic'`, a signing timestamp, and the expected leaf-certificate fingerprint. It does not assert a stable Identifier or compare designated requirements between releases.
- `scripts/test-release-wheels-macos.sh` installs the supplied wheels into a temporary virtual environment and runs `uv`, `uvx`, and `uv-build` with `--help`. It does not check signing identifiers.

These checks were read, not executed. No repository tests were added or run because the reproduced behavior is in published artifact metadata and this task permits no checkout modifications. Cryptographic signature validation, notarization assessment, macOS requirement evaluation, actual `uv self update`, Little Snitch prompts, and MDM/PPPC policy behavior remain untested here. A macOS host, the relevant security-product version, and a concrete rule or policy are needed to reproduce those downstream effects; they are not needed to reproduce the identifier changes.

## Implementation and release history

`changelogs/0.12.x.md` states that 0.12.12 introduced Developer ID signing and notarization for macOS executables in release archives and wheels. The 0.12.24 notes describe the later 16 KiB signature-page change. The 0.13.1 notes do not announce stable signing identifiers. The three related merged signing pull requests listed above were inspected alongside these release notes.

The 0.12.12 `scripts/sign-release-macos.py` invokes `rcodesign sign` without `--binary-identifier`. The checkout's `.github/workflows/sign-release-binaries.yml:160` pins `astral-sh/github-actions/sign-macos` to `a30f3520f325da7fed035847afec51629d0ef4fa`. Its `sign-macos/scripts/sign-release-macos.py` likewise omits an explicit identifier and uses `--config-file /dev/null`. This identifies the shared signing implementation to investigate; the artifact measurements, rather than the omitted option alone, establish the reproduction outcome.

## Search scope and excluded candidates

The preceding issue-context investigation searched open and closed issues and pull requests for the reported identifiers, `sh.astral.uv`, signing identifiers, designated requirements, `binary-identifier`, Little Snitch, changed program identity, firewall rules, allowlists, Santa, MDM/PPPC, and keychain approvals. It supplemented incomplete PR search results with workflow history, open PR titles, recent closed PR listings, and direct inspection of related changes. That search found no existing stable-identifier fix or duplicate. This reproduction adds independent artifact evidence; it does not claim to have repeated the full duplicate search.

- astral-sh/uv#16726 concerns uv-managed Python interpreters being killed and requests SDK/MDM details, not changing uv or uvx identities.
- astral-sh/uv#8069 concerns self-update downloads through a mirror behind a firewall.
- astral-sh/uv#22219 concerns ad-hoc signatures for edited Mach-O dylibs.
- Open signing prototypes astral-sh/uv#20660 and astral-sh/uv#20641 concern earlier wheel-signing and assembly approaches; the preceding investigation found no explicit identifier setting in their diffs.
- Closed, unmerged astral-sh/uv#21552 was superseded by astral-sh/uv#21754.

## Recommended next step

Decide on stable, distinct identities for uv and uvx and investigate configuring them in the shared macOS signing action. The reporter's `sh.astral.uv` and `sh.astral.uvx` are suggestions, not an established naming contract.

Add release verification for the chosen identifiers and compare separately built versions, including the final archives and wheels for both supported macOS architectures. Use native macOS checks to confirm that a rule based on the chosen identity continues to match after an upgrade. No completed fix or release timeline is established.

Only this temporary issue-context README and reproduction files under `/tmp` were written. No checkout files or GitHub state were changed.
