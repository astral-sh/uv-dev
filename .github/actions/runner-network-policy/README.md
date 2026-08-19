# Runner network policy prototype

Use the reviewed, SHA-pinned repository action as the first step of a disposable Linux VM job. The
action must be referenced from a repository, not through a checkout-relative path: GitHub does not
run `pre` hooks for local actions. The VM needs systemd, Python 3.11 or newer, and passwordless sudo
during setup. The bootstrap installs `nftables` from the configured Ubuntu/Debian repositories if
the image does not already contain it.

Select a bundled `profile`, explicitly set `disposable: true`, and choose `privileges: drop` or
`privileges: retain`. The default removes sudo and access to the Docker/containerd services. Jobs
using job containers, service containers, `ubuntu-slim`, macOS, Windows, or persistent self-hosted
runners are not supported by this first prototype. Privileged build jobs must retain privileges
explicitly and cannot treat the policy as tamper-resistant.

The root-owned service runs as a separate, unprivileged account. An atomic `nftables` ruleset
redirects new outbound DNS, HTTP, and HTTPS connections to it for both IP families. Other external
traffic, including forwarded container traffic, is rejected. The proxy checks DNS questions, HTTP
authorities, and the TLS ClientHello's SNI against a default-deny domain policy, resolves the
authorized hostname itself, and refuses non-public upstream addresses. Explicit HTTP proxy settings
are also exported for clients that support them. HTTPS is forwarded without decrypting it or
installing a certificate authority.

Existing runner-owned HTTPS connections from trusted bootstrap are retained by exact destination and
source port. Action downloads, runner startup, and container preparation can happen before the first
action's `pre` hook. This is therefore an early-job hardening control, not isolation of the entire
VM lifecycle. A process retaining host root, another privileged service, or a permitted
application-layer relay can bypass the intended domain boundary. Domain rules do not constrain HTTPS
paths, methods, tenants, or credentials. TLS without cleartext SNI and other network protocols fail
closed. Local loopback services remain available for tests.

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
