# Verify before wheel installation

```sh
uv run --no-project --python 3.12 --with pypi-attestations==0.0.30 \
  python scripts/research/provenance_install.py package-1.0-py3-none-any.whl \
  --attestation package.provenance.json \
  --identity 'https://github.com/OWNER/REPO/.github/workflows/publish.yml@refs/tags/TAG' \
  --issuer https://token.actions.githubusercontent.com \
  --uv /path/to/uv --target work/verified-install \
  --evidence-cache work/provenance-evidence --work-dir work
```

Supply the expected certificate identity and issuer independently of the evidence. The script
accepts a local PEP 740 attestation or a PyPI provenance object and uses
`pypi-attestations`/Sigstore to verify every contained attestation against that policy and the
wheel's name/digest. Multiple identities need a richer policy than this deliberately narrow
prototype. It does not infer trust from the provenance object's publisher fields or implement
cryptography itself.

Only successful verification reaches actual `uv pip install`. The installer gets a private snapshot
of the verified bytes and an explicit required SHA-256 hash; it installs without dependencies or
source builds into a new target directory. The script does not import the installed package. A
verified artifact can still contain malicious code, and a publish attestation need not prove which
source commit produced its bytes. This is a separate experiment, not integrated uv installer policy,
dependency-graph verification, or a malware detector.

The evidence cache key includes artifact filename/digest, evidence digest, identity/issuer policy,
verifier versions, and trust environment. Cache hits **always run cryptographic verification
again**: no acceptance verdict is stored that could outlive root rotation or verifier changes. The
established verifier owns trust material and its refresh/expiry behavior. `--offline` disables its
TUF refresh and has the verifier's corresponding trust-material limits. `--staging` is for signed
fixtures, not production artifact verification.

Output reports binary/verifier versions, the policy, cache hits, predicates, verification time, and
target. Verification failure exits before installation. No credentials, signing, browser login, or
publication are needed.

This experiment follows [PEP 740](https://peps.python.org/pep-0740/) and the
[official verifier](https://github.com/pypi/pypi-attestations).
[Sigstore](https://doi.org/10.1145/3548606.3560596) supplies the cryptographic architecture;
[Why Johnny Adopts Identity-Based Software Signing](https://www.usenix.org/conference/usenixsecurity26/presentation/kalu)
motivates evaluating explicit policy and integration costs before a default.

The experiment uses an explicitly provisioned Python environment. `pypi-attestations` is pinned in
the command; transitive dependencies use the configured registry and their installed verifier
versions appear in each result. This is not a locked dependency environment.

Run `provenance_matrix.py` in the same environment, supplying the artifact, attestation, independent
identity and issuer, `--uv`, and a new `--work-dir`. It verifies cold, warm, and repaired cache
entries, then rejects a different identity, a different issuer, changed wheel bytes, a changed
signature, and empty evidence. A final valid warm run checks that negative cases did not replace the
original policy entry. Every rejected case requires the installation target to remain absent.

The official verifier's
[signed fixture](https://github.com/pypi/pypi-attestations/tree/v0.0.30/test/assets) includes
`rfc8785-0.1.2-py3-none-any.whl` and its `.attestation`; its certificate identity is
`william@yossarian.net`, issuer is `https://github.com/login/oauth`, and verification needs
`--staging`. `--offline` uses the verifier's available trust material. The matrix accepts local
copies and performs no fixture download. Cache corruption is repaired from independently supplied
evidence and still triggers cryptographic verification.

The wheel is copied to a private snapshot while computing its digest in bounded chunks. Verification
and installation use that same snapshot, without retaining the full wheel in memory. After
verification, an atomic directory creation reserves the new target; a concurrent consumer cannot
merge into an existing target. The matrix launches two valid consumers for one target and requires
exactly one installation to succeed. Failed installations can leave their reserved target for
inspection.

For local copies of the signed fixture:

```sh
uv run --no-project --python 3.12 --with pypi-attestations==0.0.30 \
  python scripts/research/provenance_matrix.py rfc8785-0.1.2-py3-none-any.whl \
  --attestation rfc8785-0.1.2-py3-none-any.whl.attestation \
  --identity william@yossarian.net --issuer https://github.com/login/oauth \
  --uv /path/to/uv --work-dir work/provenance-matrix --staging --offline
```
