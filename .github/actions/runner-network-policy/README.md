# Runner network policy prototype

Use the reviewed, SHA-pinned repository action as the first step of a disposable Linux VM job. The
action must be referenced from a repository, not through a checkout-relative path: GitHub does not
run `pre` hooks for local actions. The VM needs systemd, Python 3.11 or newer, and passwordless sudo
during setup. The bootstrap installs `nftables` from the configured Ubuntu/Debian repositories if
the image does not already contain it.

Select a bundled `profile`, explicitly set `disposable: true`, and choose `privileges: drop` or
`privileges: retain`. The default makes the canonical `/usr/bin/sudo` executable root-only and
removes access to the Docker/containerd services. Jobs using job containers, service containers,
`ubuntu-slim`, macOS, Windows, or persistent self-hosted runners are not supported by this first
prototype. Privileged build jobs must retain privileges explicitly and cannot treat the policy as
tamper-resistant.

The root-owned service runs as a separate, unprivileged account. An atomic `nftables` ruleset
redirects new outbound DNS, HTTP, and HTTPS connections to it for both IP families. Other external
traffic, including forwarded container traffic, is rejected. The proxy checks DNS questions, HTTP
authorities, and the TLS ClientHello's SNI against a default-deny domain policy, resolves the
authorized hostname itself, and refuses non-public upstream addresses. Explicit HTTP proxy settings
are also exported for clients that support them. HTTPS is forwarded without decrypting it or
installing a certificate authority.

DNS CNAME targets discovered in an authorized answer chain are allowed for DNS resolution only, with
a bounded lifetime. They do not become permitted HTTP or TLS destinations. Unrelated answer records
and additional query records do not expand the policy.

On Depot, only the private cache/results addresses injected by the runner are redirected. Their
requests are forwarded to the corresponding public GitHub Actions service over verified TLS; this
does not grant general access to private networks or arbitrary ports. Other routes to those injected
service ports, including loopback and bridge aliases, are rejected.

Existing runner-owned HTTPS connections from trusted bootstrap are retained by exact destination and
source port. Action downloads, runner startup, and container preparation can happen before the first
action's `pre` hook. This is therefore an early-job hardening control, not isolation of the entire
VM lifecycle. GitHub can also run later `pre` hooks after an earlier hook fails, so a setup failure
before the firewall is installed is not a fail-closed runner boundary. Strict startup isolation
requires a runner-managed hook or external network boundary. A process retaining host root, another
privileged service, or a permitted application-layer relay can bypass the intended domain boundary.
Domain rules do not constrain HTTPS paths, methods, tenants, or credentials. TLS without cleartext
SNI and other network protocols fail closed. Local loopback services remain available for tests.

The post hook saves aggregate hostname/event counts but does not restore network or sudo access. The
disposable runner's trusted destruction is the cleanup boundary. Do not use this action on a machine
that needs to be reused. The proxy does not record URLs, headers, request bodies, TLS plaintext, or
credentials.

Edit `.github/runner-network-policy.toml` and run
`python3 scripts/generate_runner_network_policy.py` to regenerate the action's policy bundle. The
generator imports selected domain rules from `agents/codex/config.toml`; the bundled copy allows the
early hook to run without trusting the checked-out pull request. The job mapping records starting
profiles that still require hosted validation, not an automatically inferred minimum.

Run `python3 scripts/test_runner_network_policy.py` for local loopback integration tests. Never run
`install.py` on a development machine.

## Exact URL profiles

Set the optional `url-profile` input to a reviewed profile in `url-policies.json` to enforce HTTP
methods, paths, and query strings as well as domains. The domain profile remains an upper bound:
every URL host must be permitted by it. In this mode, new external HTTP and HTTPS connections are
request-aware; there is no unrestricted `CONNECT` tunnel or fallback to domain-only TLS forwarding.

Each rule names an HTTP or HTTPS URL, allowed `methods`, an `exact`, directory `prefix`, or
`{integer}`-segment `template` path match, and an `exact` or explicitly unrestricted `any` query
match. An exact rule can set `allow_repeated_slashes: true` for a server-issued path with that
spelling; prefix and template rules cannot use this exception. Other ambiguous path spellings,
underscore-bearing request headers, unsupported framing, and protocol upgrades are rejected.
Redirects are returned to the client, so the next URL must pass the policy separately. Rules govern
requests, not the behavior of an allowed service: a permitted application-layer relay or an overly
broad prefix can still widen access.

The `github-api-probe` example allows only `GET` and `HEAD` for the uv-dev repository API and
GitHub's rate-limit endpoint. Its explicit `runner_services: true` setting also adds the job's
validated runtime API prefixes, the exact runner-provided OIDC route, Run Service renewal/completion
routes on GitHub's named hosts, results/log and artifact RPCs, GitHub's named results storage
accounts, and the reviewed hosted-runner control-plane hosts. These are visible in the root-owned
effective `url-policy.json`. They are coarse infrastructure exceptions, not inferred minimal
permissions. The Run Service exception permits only `POST` to the two named routes, optionally under
a numeric shard path; GitHub does not expose that original service base to actions in every job. The
cache RPCs are not among the generated service rules. A profile can omit `runner_services` when
those exceptions are not needed, but this can interrupt GitHub's job reporting.

URL mode installs a short-lived local CA on the disposable VM and terminates TLS 1.2 or newer using
HTTP/1.1. The proxy verifies upstream TLS itself. The leaf certificate covers only the exact allowed
hostnames, and the CA signing key is deleted before the service starts. Root owns the executable,
policy, and certificate metadata; only the unprivileged service can read its leaf key and write its
separate audit directory. URLs, query strings, plaintext, credentials, and request bodies are not
logged. Clients with separate trust stores, certificate pinning, HTTP/2-only protocols, or streaming
request bodies may fail closed until explicitly supported. As with domain mode, trusted VM teardown
is the cleanup boundary, and a first action `pre` hook does not isolate earlier runner setup.

For example:

```yaml
- uses: astral-sh/uv-dev/.github/actions/runner-network-policy@<reviewed-commit>
  with:
    profile: github
    url-profile: github-api-probe
    disposable: true
```

Run `python3 -m unittest discover -s scripts -p 'test_runner_url_*.py'` for the focused URL-policy
tests. Do not enable this prototype in a publishing job before its required URLs and clients have
been exercised on that job's runner.
