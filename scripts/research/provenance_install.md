# Verify before wheel installation

```sh
uv run --script scripts/research/provenance_install.py package-1.0-py3-none-any.whl \
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
target. Failure exits before installation. No credentials, signing, browser login, or publication
are needed.

This experiment follows [PEP 740](https://peps.python.org/pep-0740/) and the
[official verifier](https://github.com/pypi/pypi-attestations).
[Sigstore](https://doi.org/10.1145/3548606.3560596) supplies the cryptographic architecture;
[Why Johnny Adopts Identity-Based Software Signing](https://www.usenix.org/conference/usenixsecurity26/presentation/kalu)
motivates evaluating explicit policy and integration costs before a default.
