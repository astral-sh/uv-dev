# CSOAI — uv AI governance Python package manager

Issue: astral-sh/uv#21690

Classification: question

## Summary

The report consists of a title describing CSOAI as an “uv AI governance Python package manager”
and a body naming the `csoai-gspc-mcp` npm package and the CSOAI GSPC harness repository. It does
not identify a uv command or subsystem, describe expected and actual behavior, provide an error or
reproduction, or request a specific integration or feature.

The linked project's public metadata describes a Python AI-governance measurement harness and an
MCP server. That establishes what the linked project is, but not what change or support is sought
from uv. No existing uv issue or pull request was found that tracks the same unstated request.

## Reproduction

Outcome: `needs_more_information`.

No meaningful behavioral reproduction can be constructed from the report. It contains no uv
command, project or workspace configuration, dependency specification, platform, uv or Python
version, error output, expected behavior, or actual behavior. The installed environment has uv
0.12.14 on x86_64 Linux with Python 3.12.3, but running an arbitrary command in that environment
would not test a reported claim. A repository search also found no references to `CSOAI`,
`csoai-gspc-mcp`, `gspc-harness`, or “AI governance” that supply an implied uv workflow or existing
test case.

To investigate further, the reporter needs to provide the exact uv command and inputs, a minimal
`pyproject.toml` or other relevant configuration, the uv and Python versions and platform, the
observed output or error, and the expected result. If the report is instead a feature proposal,
it needs to state the requested capability and how it should integrate with uv; no behavioral
reproduction applies until that scope is defined.

## Draft response

Thanks for the links. This report does not describe an issue with uv or a specific change you would
like uv to make. Could you clarify the uv-related request, including the uv command or workflow
involved, the expected behavior, and what currently happens? If this is a feature proposal, please
describe the capability and how it would integrate with uv.

## Classification

This is classified as a question because no incorrect uv behavior has been alleged and no concrete
new capability has been requested. Additional clarification is required before the report can be
evaluated as a bug or enhancement. The existence and stated purpose of the linked MCP server and
Python harness do not by themselves establish a uv integration request.

It is not a duplicate. The closest repository discussions have explicit scopes that are materially
different from the content of astral-sh/uv#21690, and there is not enough information to infer that
the reporter intends any of those requests.

## Related

No meaningfully related issue or pull request was found. astral-sh/uv#12500 requests that uv expose
native MCP tools for common uv commands, while astral-sh/uv#21690 merely names an external MCP
package. astral-sh/uv#12457 requested dedicated CI installation coverage for an MCP-related Python
package and was closed after maintainers found no package-specific compatibility concern;
astral-sh/uv#21690 reports no installation or compatibility failure. astral-sh/uv#18717 requests a
global package denylist for machine- or organization-wide dependency policy; its use of
“governance” concerns package installation controls, not the AI-governance measurement described by
the linked CSOAI project. AI-assistant configuration and documentation discussions
astral-sh/uv#19677, astral-sh/uv#13901, and astral-sh/uv#18304 likewise address different concrete
features.

## Search and evidence

The report was decomposed into the exact identifiers `CSOAI`, `csoai-gspc-mcp`, and
`gspc-harness`; the literal phrase `AI governance`; the MCP subsystem implied by the npm package;
and the broad package-governance concept implied by the title. There were no uv commands,
versions, platforms, errors, triggering conditions, expected results, or actual results to search.

Authenticated searches covered open and closed issues and open, closed, and merged pull requests.
Literal searches used each exact identifier and phrase. Conceptual searches included MCP, Model
Context Protocol, AI agents, governance, dependency policy, policy enforcement, package policy,
package allow/deny controls, supply chain, SBOM, and Python package manager terminology. Searches
for fixes and pull requests used the same exact and conceptual terms. Exact searches returned only
astral-sh/uv#21690 and no pull requests. The closest conceptual candidates were inspected with
their comments and references; none described the same observable behavior or requested
capability, and none identified a canonical discussion for the content of this report.
