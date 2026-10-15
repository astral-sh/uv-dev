# `uv publish --index <name>` does not reuse `UV_INDEX_<NAME>_USERNAME` for keyring authentication

Issue: astral-sh/uv#22588

Classification: duplicate

## Summary

The reporter uses uv 0.13.1 on Windows with Azure Artifacts, artifacts-keyring, and the subprocess keyring provider. A named index supplies both its simple-index URL and publish URL. Its username is configured through `UV_INDEX_<NAME>_USERNAME`, and package downloads succeed.

Running `uv publish --index <NAME> --keyring-provider subprocess` nevertheless prompts for a username. The reporter observes a 401 after entering the PyPI token username; explicitly supplying the Azure keyring username through `--username` or `UV_PUBLISH_USERNAME` succeeds. The requested capability is to reuse the selected index's configured username before prompting.

astral-sh/uv#9845 is the canonical open discussion, with astral-sh/uv#10193 already closed as its duplicate. astral-sh/uv#9227 explains the adjacent keyring/prompt interaction, and astral-sh/uv#9694 introduced named-index publishing.

## Draft response

This is already tracked in astral-sh/uv#9845. `--index` selects the publish and check URLs, but the upload username is not populated from `UV_INDEX_<NAME>_USERNAME`. That discussion remains open, including concerns about download and upload credentials differing.

For your Azure keyring setup, continue using `--username VssSessionToken` or `UV_PUBLISH_USERNAME=VssSessionToken` with the subprocess provider enabled. Please add your username-only keyring case to astral-sh/uv#9845 so the discussion stays together.

## Classification

astral-sh/uv#9845 already tracks the same missing named-index credential fallback during publishing and remains open. The username-only Azure keyring trigger is a more specific instance of that request. Current source confirms that index selection supplies URLs without populating the upload username. No evidence establishes a previously fixed upload-username regression; astral-sh/uv#15546 fixed the separate check-URL credential path.

The duplicate classification takes precedence over deciding whether the underlying missing fallback is a bug or enhancement. The canonical issue is labeled enhancement and needs-decision. Its discussion initially considered a low-priority index-credential fallback, but a later maintainer comment raised the risk of silently selecting download credentials for an upload. Do not present the earlier suggestion as an agreed implementation plan or promise the reporter's requested precedence.

The new report identifies a particular username-only use case: the password would still come from keyring. That distinction is useful to the existing design discussion, without requiring a separate canonical issue. The reporter does not identify an earlier working version.

## Related

- astral-sh/uv#9845 — issue, open: Read `UV_INDEX_<NAME>_` credentials during `uv publish`. Canonical discussion of the same missing credential fallback with `uv publish --index`. It explicitly requests reading named-index username/password environment variables. Maintainers discussed fallback precedence, then raised concerns about download and upload credentials differing; it remains labeled enhancement and needs-decision. This report adds a username-only Azure keyring case.
- astral-sh/uv#10193 — issue, closed: How to provide publish credentials for an alternative (named) package index?. Named-index publishing failed when only `UV_INDEX_<NAME>_USERNAME` and its password counterpart were supplied. A maintainer and the reporter identified it as a duplicate of astral-sh/uv#9845. It involves Artifactory and complete credentials rather than Azure keyring, but confirms the same canonical discussion.
- astral-sh/uv#9227 — issue, open: Allow `--token` to be empty if `--keyring-provider` is set with `uv publish`. Adjacent keyring/prompt behavior: publishing works with an explicit username, while prompting does not provide the expected keyring flow. A maintainer explains that keyring lookup requires a username and that missing credentials trigger prompting. It concerns PyPI token handling, not reuse of named-index environment variables.
- astral-sh/uv#9694 — pull request, merged: Publish: Support --index <name>. Introduced named-index publishing and resolution of publish/check URLs, merging on 2024-12-10. astral-sh/uv#9845 reported missing credential reuse immediately afterward in uv 0.5.8. This is the originating feature, not a fix for username inheritance.

## Supporting evidence

### Current source and documentation

Reviewed checkout commit `9385290` dated 2026-10-15.

- `crates/uv-publish-commands/src/lib.rs:54`: named-index selection resolves `publish_url` and `check_url`; it does not populate the upload username from the selected index.
- `crates/uv-publish-commands/src/lib.rs:141`: upload credential gathering receives the separately supplied username and password. The named index's authentication configuration is not passed into this function.
- `crates/uv-publish-commands/src/lib.rs:289`: the documented username sources include publish-specific CLI/environment settings, the publish URL, token/trusted-publishing handling, and prompting; named-index environment variables are absent.
- `crates/uv-publish-commands/src/lib.rs:365`: after trusted-publishing handling, missing username and password lead to interactive prompting. At line 410, eager keyring lookup uses the publish URL and an already available username when no password is present. This supports why supplying a publish username changes the flow.
- `crates/uv-publish-commands/src/lib.rs:161`: the check-URL registry client separately receives the index locations and keyring provider. Upload authentication and index-read authentication are distinct paths.
- `crates/uv-cli/src/lib.rs:7469`: the upload username option is connected to `UV_PUBLISH_USERNAME`.
- `docs/guides/integration/azure.md:54`: Azure keyring guidance requires the conventional username `VssSessionToken` and demonstrates the index-specific username variable. The publishing section, starting at line 77, says to configure publish credentials “if not using keyring” without showing a publish username for that keyring case. This helps explain the expectation, but does not establish that fallback is implemented.

The source establishes the missing username handoff and prompt ordering. The reported Azure 401 and successful workaround were not independently exercised; no publish, keyring access, build, or credential-dependent reproduction was run.

### Maintainer discussion and historical checks

In astral-sh/uv#9845, the 2024-12-17 maintainer comment proposed index credentials as the lowest-priority source. The 2025-01-28 comment reconsidered that approach because download and upload credentials can differ and an automatic fallback could fail instead of prompting. The issue remains open. In astral-sh/uv#10193, both the maintainer and reporter explicitly agreed to centralize the same named-index credential question in astral-sh/uv#9845.

astral-sh/uv#9694 merged on 2024-12-10 and introduced URL selection through named indexes. The canonical report followed on 2024-12-12 against uv 0.5.8. This history supports a longstanding missing capability rather than a newly established uv 0.13.1 regression.

## Other candidates considered

- astral-sh/uv#10386 has a very similar title and was also closed as a duplicate of astral-sh/uv#9845, but its reproduction concerns reading the check URL with separate upload credentials. It supports the discussion chain without being the best symptom match.
- astral-sh/uv#18003 concerns prompting despite stored credentials, and astral-sh/uv#17608 requests a default token username. Neither specifically supplies a username through a named-index environment variable; a PyPI token default would not supply Azure's required username.
- astral-sh/uv#9997 was resolved by exporting shell variables. Here the request concerns a different variable namespace, and the source confirms the missing handoff.
- astral-sh/uv#11836 and astral-sh/uv#15587 concern failed authentication while checking for already-published files. Merged astral-sh/uv#15546 moved index-credential caching into the registry client builder. That fix does not establish upload-username inheritance. astral-sh/uv#16236 similarly concerns stricter check-URL authentication and was resolved by configuring index-read credentials.
- Closed, unmerged astral-sh/uv#8806 proposed keyring documentation, while merged astral-sh/uv#8827 added warnings for missing keyring entries. Neither implements the requested named-index username fallback.

## Search scope

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using authenticated gh. Literal searches covered UV_INDEX_<NAME>_USERNAME, UV_INDEX with publish, VssSessionToken, Enter username, and publish with 401 Unauthorized. Conceptual searches covered named-index publishing, credential reuse, private-index prompts, Azure authentication, and keyring username selection, including searches without platform/provider details. Fix-oriented searches covered publish credentials, username, and keyring; PR searches returned no additional candidates, so referenced PRs were inspected directly. Read candidate comments and followed astral-sh/uv#10193 and astral-sh/uv#10386 to astral-sh/uv#9845, plus its originating PR. Ruled out astral-sh/uv#18003 and astral-sh/uv#17608 as closer matches because they concern stored-credential selection/default token usernames; astral-sh/uv#9997 involved unexported variables. Inspected astral-sh/uv#11836, astral-sh/uv#15587, astral-sh/uv#16236, and merged astral-sh/uv#15546: these concern check-URL authentication rather than the upload username. Also inspected astral-sh/uv#8806 and astral-sh/uv#8827 from the keyring discussion; neither implements named-index username reuse. Compared these findings with the current publish implementation and Azure integration documentation.

## Handoff

Centralize the report in astral-sh/uv#9845 and retain the username-only Azure keyring scenario as evidence for that discussion. The immediate supported workaround is the publish-specific username with subprocess keyring enabled. Any fallback design needs to address credential precedence and the separation between reading and uploading.

This handoff and the structured result are drafts for maintainer review. No GitHub changes were made. Only this temporary issue-context README was authored; the checkout's existing modifications were left unchanged.
