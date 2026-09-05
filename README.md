# Update README with Agent-Friendly Score badge

Issue: astral-sh/uv#21482

Classification: duplicate

## Summary

The reporter asks uv to add a badge from Agent Friendly Code to the root README. The badge would
display the service's 81.7/100 assessment and link to its report for uv.

The closest repository precedent is astral-sh/uv#7883, where another external service asked uv to
add its promotional badge and a maintainer declined. Although the service and advertised metric are
different, both issues require the same repository and policy decision: whether uv should display an
external service's badge in its README. The current request can therefore be centralized with that
closed discussion.

The request has now also been explicitly declined on astral-sh/uv#21482 by a repository member.

## Maintainer decision

Repository member `zsol` responded “No thank you” on September 5, 2026. This directly confirms that
the proposed Agent Friendly Code badge will not be added; no further technical investigation or
implementation is needed.

## Classification

`duplicate` fits because astral-sh/uv#7883 covers the same underlying request: an external service
operator asking uv to place that service's badge in the project README. Its maintainer response is
the existing repository decision most directly applicable here. The different provider and score
do not change the requested repository behavior.

The repository-member response on astral-sh/uv#21482 independently confirms the outcome for this
specific badge request. It does not change the `duplicate` classification because
astral-sh/uv#7883 remains the earlier canonical discussion of the same class of request.

This is not a bug because no incorrect uv behavior is reported. If considered without the prior
discussion, it would be an enhancement because it requests new README content, but duplicate takes
precedence under the triage rules.

## Related

- astral-sh/uv#7883 — **HelloGitHub Badge** (closed issue). This is the canonical prior discussion:
  another external service requested its badge in uv's README, and a maintainer replied that the
  project would pass.
- astral-sh/uv#15076 — **Update badge logo SVG** (open issue). This is adjacent but materially
  different: it concerns uv's project-owned badge asset for downstream users to embed in their own
  READMEs, rather than adding an external rating badge to uv's README.
- astral-sh/uv#15075 — **Update badge logo SVG** (open pull request). This is the implementation
  paired with astral-sh/uv#15076; its body and maintainer discussion confirm that its scope is the
  existing uv badge's logo, not external scoring or promotion.

## Supporting evidence

The root README currently displays PyPI version, supported-Python, and Discord badges, but no
third-party codebase rating. Exact searches for “Agent Friendly,” `agentfriendlycode.com`, “score
badge,” and “agent friendliness” found no pre-existing issue or pull request for this provider beyond
astral-sh/uv#21482 itself.

Broader searches covered “badge,” “README badge,” “add badge README,” “external badge,” “code quality
badge,” “AI friendly,” and `shields.io` across open and closed issues and open, closed, and merged
pull requests. The strongest candidates were then inspected with their comments and references.
astral-sh/uv#13061 was ruled out because it was generated from an off-topic CI-badge comment on the
unrelated astral-sh/uv#13051. astral-sh/uv#18304 was also ruled out because it proposed an AI chat
feature for the documentation rather than a README badge. No merged pull request implementing the
requested external badge was found.
