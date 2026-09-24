# uv no longer exports `FlatDistributions`

Issue: astral-sh/uv#21964

Classification: enhancement

## Summary

The reporter integrates uv's Rust crates into pixi and previously used the public
`FlatDistributions` type to construct a `VersionMap`. They report that this path stopped compiling
after astral-sh/uv#21540 and ask for the type to be public again.

The cited pull request is the direct source-confirmed change. It made `FlatDistributions`
crate-private, removed its re-export from `uv_resolver`, and removed the conversion from
`BTreeMap<Version, PrioritizedDist>`. The change also moved flat-distribution ranking into the
resolver so tags, hash policy, and build options are applied for each resolver request. The current
source still exports `VersionMap` but keeps `FlatDistributions` and its conversion into
`VersionMap` private.

No other issue or pull request was found that tracks restoring this type or providing a replacement
way to construct a `VersionMap`. astral-sh/uv#2015 is a close but distinct precedent: a different
Rust API used by pixi was made crate-private and then restored by astral-sh/uv#2016.

## Draft response

Thanks for flagging this. astral-sh/uv#21540 intentionally moved flat-distribution ranking into the
resolver so that tags, hash strategy, and build options are applied per resolver request; as part of
that change, `FlatDistributions` became crate-private and its public re-export and `BTreeMap`
conversion were removed. The `uv-resolver` Rust interface is internal and does not carry stability
guarantees, so exposing the old type again may not be the right API now that its role has changed.
Could you link the current pixi call site and show which input you need to convert into a
`VersionMap`? That will let us evaluate whether `FlatDistributions` should be exposed again or
whether a narrower constructor would fit the new policy model.

## Classification

This is an enhancement request. The repository evidence shows that astral-sh/uv#21540 deliberately
internalized distribution ranking rather than accidentally omitting an otherwise unchanged export.
The reporter is asking for a Rust construction capability to be exposed again, potentially through
a replacement API suited to the new resolver-policy model. The repository's versioning policy also
states that crates other than `uv`, `uv-build`, and `uv-version` provide no stability guarantees and
that their Rust interfaces are internal and unstable. There is no report of incorrect behavior from
a supported uv command.

This is not a duplicate: no existing open issue or pull request tracks the same request. It is not
classified as a bug because the visibility and conversion changes were intentional, and
`uv-resolver` does not promise a stable Rust API. The previously public API and the close precedent
make the request concrete, but do not establish that the current internal visibility is incorrect.

## Related

- astral-sh/uv#21540 — “Apply flat index policies in each resolver” (merged pull request). This is
  the direct cause. Its diff changes `FlatDistributions` from `pub` to `pub(crate)`, removes the
  public re-export and external `BTreeMap` conversion, and constructs ranked flat distributions
  inside the resolver with the active tags, hash strategy, and build options.
- astral-sh/uv#2015 — “`Intepreter::query` was made `pub(crate)`” (closed issue). This is an adjacent
  precedent from the same pixi integration: another formerly public uv Rust API became
  crate-private and was restored by astral-sh/uv#2016. It concerns `Interpreter::query`, not
  `FlatDistributions` or `VersionMap`, so it is not a duplicate.

## Supporting evidence

- The body and commits of astral-sh/uv#21540 describe ranking raw `FlatIndex` entries inside each
  resolver so callers can share an index while applying different policies. One commit is titled
  “Keep flat distribution ranking internal to the resolver.”
- The current source declares `FlatDistributions` as `pub(crate)`, keeps
  `From<FlatDistributions> for VersionMap` internal through the private type, and publicly re-exports
  `VersionMap` but not `FlatDistributions`.
- The versioning policy documents `uv-resolver` among the crates whose Rust interfaces are internal,
  unstable, and versioned as `0.0.x`.
- Literal searches covered `FlatDistributions`, `VersionMap`, “flat distribution,” and the claim
  that the export disappeared. Conceptual searches covered `FlatIndex`, public resolver APIs,
  crate-private API changes, external consumers, programmatic uv-crate use, and pixi. Searches were
  run across open and closed issues and open, closed, and merged pull requests.
- Fix-oriented inspection covered astral-sh/uv#21540 and its referenced follow-ups. No restoration
  fix or replacement constructor was found. astral-sh/uv#21223 was inspected because
  astral-sh/uv#21540 calls itself a follow-up, but it concerns locked-source hash enforcement rather
  than public API visibility. astral-sh/uv#2015 was inspected as the strongest conceptual candidate
  and ruled out as a duplicate because it concerns a different type and was resolved by
  astral-sh/uv#2016.
