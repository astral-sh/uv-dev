# Remove `HashDigestWire`

Issue: astral-sh/uv#21673

Classification: enhancement

## Summary

This issue requests a deferred internal serialization cleanup after the `HashDigest` refactor in
astral-sh/uv#21139. `HashDigestWire` exists to retain the current Serde and rkyv cache layout while
`HashDigest` moves to validated, size-specific variants. Once all cache buckets that contain this
data—source distributions, flat indexes, simple-index responses, and wheels—are being bumped for
another reason, the compatibility type, conversions, Serde adapter, and custom rkyv verification can
be removed in favor of the new representation.

astral-sh/uv#21139 is the only close existing item. It is the direct antecedent and deliberately
retains `HashDigestWire`; it does not implement or independently track its eventual removal. No
existing issue or pull request was found that can serve as a duplicate.

## Draft response

astral-sh/uv#21139 confirms that `HashDigestWire` is intentionally retaining the current cache wire
layout to avoid cache churn from the `HashDigest` refactor. We can keep this open as the follow-up
for the next change that bumps all affected cache buckets. At that point, the compatibility
conversions and custom rkyv verification can be removed in favor of the new representation, while
`CachedHashDigests` should remain separate and the `Blake2b` spelling should change only after
checking every remaining serialized consumer.

## Classification

This is an enhancement because it asks for an internal refactor that simplifies serialization when
a later cache-format change makes the compatibility layer unnecessary. The report does not describe
incorrect user-visible behavior or a current correctness failure, so it is not a bug. It is not a
duplicate of astral-sh/uv#21139: that pull request introduces the validated hash representation and
explicitly preserves the old wire layout, while this issue tracks cleanup deferred until a future
cache-bucket bump.

## Related

- astral-sh/uv#21139 — **Open pull request, “Refactor HashDigest APIs.”** This is the direct
  antecedent. Its body says the original approach bumped several cache buckets but was changed to
  preserve the old layout through `HashDigestWire`. Maintainer discussion rejects bucket churn for
  an internal refactor and recommends dropping the wire type when a cache bump is needed for
  user-facing functionality. Its diff defines the compatibility type and adds a cache-bucket TODO
  for its later removal.

## Supporting evidence

Literal searches covered `HashDigestWire`, `HashDigest`, `CachedHashDigests`, `Digest<N>`,
`Blake2b`/`Blake2b256`, rkyv, and the Serde/archive identifiers. Conceptual searches covered cache
wire compatibility, cache churn, cache-format and bucket versioning, schema migration,
serialization invariants, and invalid hash states across open and closed issues and open, closed,
and merged pull requests.

The strongest adjacent candidates were inspected but do not track this work:

- astral-sh/uv#21131 prompted the broader hash API work through CycloneDX export behavior, but it
  neither changes cache wire compatibility nor requests this cleanup.
- astral-sh/uv#6145 requests reduced repeated `File` data for cache performance. That is a separate
  storage optimization, consistent with keeping `CachedHashDigests` out of this cleanup.
- astral-sh/uv#19298 reports a fixed regression in deserializing legacy cache IDs. It concerns an
  incompatible ID-length change and a user-visible hard failure, not the planned hash wire-format
  transition here.
- astral-sh/uv#10483 documents the general policy of using versioned buckets for incompatible cache
  changes, while astral-sh/uv#509 concerns bucket naming. Neither tracks hash serialization.

Repository evidence supports the requested timing but not a specific future bucket bump. Before
removing the compatibility layer, the implementation must confirm that every affected bucket is
changing and audit remaining serialized consumers before changing the `Blake2b` spelling.
