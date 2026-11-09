# Allow UV without using PEP 714/658

Issue: astral-sh/uv#23150

Classification: enhancement

## Summary

The reporter requests an option to bypass standalone distribution metadata advertised through
PEP 658/714. The concrete use case is Pixi's mirror middleware: resolution reads the upstream
index, while file requests are redirected to a mirror that does not serve the advertised
`.whl.metadata` files. The linked reproduction fails with HTTP 404 while fetching metadata;
it does not show a wheel installation failure or require a particular package or platform.

uv already supports indexes that do not advertise standalone metadata. This proposal adds a way
for an embedding application to override an advertisement that does not match its substituted
file host. No existing uv duplicate or implementing pull request was found.

The linked patch adds a default-enabled `use_dist_info_metadata(bool)` setting to
`RegistryClientBuilder`, carries it into `RegistryClient`, and uses it to select the existing
wheel-reading path. It proposes a Rust API setting, not a CLI flag or configuration-file option.
The current issue body also proposes silently falling back to wheel-based metadata retrieval
when standalone metadata cannot be fetched. This is an alternative requested behavior, not
implemented or accepted functionality.

Maintainer woodruffw questions whether bypassing advertised metadata belongs in uv and suggests
fixing Pixi's mirroring instead. That feedback explicitly assumes Pixi controls its index
responses; the supplied discussion does not establish that assumption.

## Maintainer feedback and open questions

In astral-sh/uv#23150, woodruffw's November 9 comment raises installer/tool/index consistency
and compatibility as reasons to question the proposed bypass. The maintainer suggests the problem
may belong in Pixi's mirroring, explicitly conditional on Pixi controlling its own index responses.
This is an initial objection and suggested investigation direction, not a final rejection or a
confirmed root cause in Pixi.

The existing reproduction reads the upstream index and rewrites artifact URLs. Clarify whether
Pixi can adjust the metadata advertisements seen by uv, or otherwise keep index advertisements
consistent with the selected file mirror. Do not assume that Pixi owns or serves the upstream
index. This architectural question should precede deciding whether uv needs a new override.
The comment adds no reproduction steps, version information, or demonstrated workaround.

## Classification

The request adds an explicit uv-client control to ignore advertised PEP 658/714 metadata for Pixi's URL-rewriting mirrors. Source confirms uv already reads metadata from wheels when sidecars are not advertised, and propagates failures when advertised sidecars cannot be fetched. The supplied failure involves inconsistent index/mirror capabilities, not established incorrect handling of a conforming index. No existing uv issue or PR was found tracking this override, and no uv regression was established; the downstream Pixi and mirror issues have separate implementation scopes.

The existing enhancement label agrees with this assessment. The external reports support the
index/file-host mismatch; they do not demonstrate a regression in uv's PEP 714 parsing.
The explicit override and the proposed silent fallback require a maintainer decision. Current
feedback first questions whether either workaround belongs in uv rather than Pixi. If an
override is considered, its whole-client versus narrower scope remains undecided. The linked
patch is a proposal, not accepted upstream functionality.

## Related

- prefix-dev/pixi#7479 — Open, “Allow users to disable PEP 714.” Direct downstream report: Pixi rewrites files.pythonhosted.org URLs to a mirror that returns 404 for an advertised .whl.metadata file. Its comments distinguish this from using a mirror's own index and explicitly propose upstreaming the small uv-client builder-setting patch. It tracks Pixi integration, not an existing uv implementation request.
- tuna/issues#1775 — Open, “为 pypi 镜像实现 PEP 658 和 PEP 714，在 API 中提供包元数据.” Tracks PEP 658/714 support in the mirror used by the reproduction. Comments initially report deployment, then clarify that the deployment was rolled back; the later missing-sidecar report tuna/issues#2606 confirms that history. This is the server-side capability, distinct from the requested uv-client override.
- astral-sh/uv#17943 — Closed, “Feature Request: Add option to disable lazy wheel (Range requests) for misconfigured servers returning Content-Length: 0 in HEAD response.” Adjacent precedent for an explicit metadata-fetching compatibility bypass: maintainers declined a switch to work around incorrect server headers, citing maintenance and performance costs. It concerns HTTP range extraction when PEP 658 metadata is absent, whereas this request skips advertised standalone metadata; it is not a duplicate or a decision on this specific API.

## Supporting evidence

- In prefix-dev/pixi#7479, the example maps `https://files.pythonhosted.org/packages` to
  `https://mirrors.tuna.tsinghua.edu.cn/pypi/web/packages`. The reported failure is a 404
  for `polars-1.44.2-py3-none-any.whl.metadata`. The reporter explains that choosing the
  mirror's own index works, but changes artifact URLs in the lockfile and does not meet the
  desired mirror behavior.
- Pixi maintainer lucascolley asks for the small uv patch and suggests proposing the API
  addition upstream. The patch leaves standalone metadata enabled by default and gates the
  existing PEP 658 branch. It has not been applied to this checkout.
- `crates/uv-client/src/registry_client.rs:1182` selects standalone metadata when
  `file.dist_info_metadata` is present, appends `.metadata` to the wheel URL, and
  propagates the cached fetch result. At line 1243, the alternative path reads the wheel
  with HTTP range requests, with streaming fallback for unsupported range requests.
  `RegistryClientBuilder` has no corresponding standalone-metadata opt-out.
- `crates/uv-client/src/html.rs:238` recognizes both `data-core-metadata` and
  `data-dist-info-metadata`, so the report is not explained by missing PEP 714 field support.
- tuna/issues#2606 reports the same Pixi mirror mapping and missing metadata error. A mirror
  maintainer says support had been added and rolled back after update problems, and points
  to tuna/issues#2414. In that earlier issue, a maintainer reports rolling back the faulty
  PEP 658/714 introduction and repairing the repository contents. These are mirror-side
  changes, not evidence that a uv fix regressed.
- In astral-sh/uv#17943, maintainers explain that uv relies on server headers, warn about
  silent performance degradation, and decline the range-request bypass. That decision is
  relevant context, but it does not settle this different standalone-metadata API proposal.

## Search coverage and ruled-out candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Literal searches covered PEP 714/658, core-metadata, dist-info-metadata, .whl.metadata, 404 Not Found, use_dist_info_metadata, and disable-pep714. Separate capability and symptom searches used disable, bypass, ignore, fallback, standalone metadata, sidecar, mirrors, and missing metadata, without package/version/platform restrictions. Inspected candidate bodies and comments, the reporter's patch, and the Pixi-to-TUNA reference chain. PR keyword searches returned no candidates; issue timelines exposed merged historical fixes that were inspected. Ruled out astral-sh/uv#3689 and fixes astral-sh/uv#3697 and astral-sh/uv#3698 (PEP 714 field-name support), astral-sh/uv#12383 and astral-sh/uv#12384 (URL fragments), and astral-sh/uv#18998 (range detection). None implements this override or establishes a regression.

The closest misleading title match, astral-sh/uv#3689, concerns recognizing the renamed
PEP 714 fields. Its merged fixes astral-sh/uv#3697 and astral-sh/uv#3698 date to May 2024
and do not disable advertised metadata or recover from missing sidecars.
astral-sh/uv#12383 and its March 2025 fix astral-sh/uv#12384 remove URL fragments from
HTML-index wheel URLs, rather than handling unsupported mirror metadata.
astral-sh/uv#18998 and astral-sh/uv#8580 discuss range-request detection or performance
after entering the wheel-reading path. astral-sh/uv#17047 concerns a proxy's range support.
These do not track the requested override.

Additional 404/index candidates were inspected: astral-sh/uv#17847 was resolved at the private
index and concerns wheel downloads; astral-sh/uv#18049 was caused by an invalid wheel filename;
astral-sh/uv#17619 concerns negative caching of package-index responses. None describes this
metadata-sidecar bypass.

## Handoff

First establish how Pixi obtains index responses and whether its mirroring layer can keep
metadata advertisements consistent with the selected artifact host. Evaluate the suggested
Pixi-side fix against that architecture; index-response control is an explicit, unverified
assumption in the maintainer feedback.

If that investigation leaves a need for uv changes, review whether the Rust client should
expose an explicit standalone-metadata override or support the reporter's alternative of silent
fallback after a failed metadata fetch. Neither has been accepted. Keep any API decision
separate from future CLI or per-index configuration design. The supplied patch and downstream
reproduction already explain the requested behavior; no additional package-specific
reproduction is needed to classify it.

Only this issue-context README was updated. No GitHub changes, checkout edits, builds, or tests
were performed.
