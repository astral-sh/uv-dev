# Disable GitHub Actions caches

Place the reviewed, SHA-pinned action first in each release job. The repository reference lets
GitHub load the action before checkout, including on runners too old for `$/` action references, and
its `pre` hook runs before later action hooks. The shared binary-build workflow can pass
`enabled: ${{ !inputs.allow-cache }}` to preserve ordinary CI caching.

The action starts a local TLS proxy for the GitHub cache service hostnames. It rejects the legacy
cache API and v2 `CacheService`, while forwarding artifact traffic. On Depot Linux runners it also
redirects the injected private HTTP cache endpoints. Linux and macOS block direct TCP connections to
the original resolved service addresses, except from the proxy. Linux also rejects forwarded
container traffic to those addresses and Depot's private cache endpoints. Windows provides DNS-based
interception without that additional firewall rule. Installation and cache-read denial are checked
before subsequent actions may run.

This is defense in depth against a trusted action restoring a poisoned cache, not a sandbox for
malicious actions. A privileged process can undo the local policy. Other cache services, remote
Docker builders, previously issued blob URLs, new service addresses, and remote container networking
are not covered. Keep `ACTIONS_CACHE_MODE: none` and explicit client opt-outs in place.

The proxy handles runtime authorization in memory. It never logs request headers, bodies, or signed
URLs. Its temporary signing key is deleted after creating the service certificate. The post hook
removes the networking and trust-store changes.
