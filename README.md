# Feature request: `uv auth` supports pluggable token provider for short-lived tokens

Issue: astral-sh/uv#22384

Classification: duplicate

## Summary

The reporter requests a locally registered external command that supplies short-lived tokens during
authentication, avoiding shell wrappers and custom Python keyring backends. The proposed interface is
`uv auth login --token-provider`; command output would supply the password for HTTP Basic
authentication, with a default `__token__` username.

astral-sh/uv#7660 is the canonical discussion, with astral-sh/uv#19025 covering the same provider and caching request. Draft PRs astral-sh/uv#19611 and astral-sh/uv#18907 address narrower credential retrieval and storage work; astral-sh/uv#18541 tracks the separate Bearer-auth capability.

## Draft response

Let's consolidate this in astral-sh/uv#7660, which already tracks command-based dynamic credentials without keyring, including your local-only uv auth proposal. astral-sh/uv#19025 also covers pluggable providers and token-expiry caching.

Please add these configuration and security constraints to astral-sh/uv#7660 before starting an implementation. It remains marked needs-design and needs-decision, so the interface still needs agreement. astral-sh/uv#18907 changes native credential storage; it does not implement token providers.

## Classification

astral-sh/uv#7660 already tracks command-based dynamic credentials without keyring, including this reporter's local-only uv auth configuration proposal; astral-sh/uv#19025 also covers pluggable commands and token-expiry caching. The additional security and storage details refine the same unresolved feature request. No regression is reported, so duplicate takes precedence over enhancement.

The key evidence is broader than a shared title: the original astral-sh/uv#7660 discussion explicitly
asks for an AWS-style `credential_process` without keyring setup. The same reporter's September 25,
2026 comment then proposes a token helper registered through `uv auth`, with no configuration shipped
in project code. Those are the central capability and trust boundary of this report.

The proposal remains a design request. It does not provide a concrete failing invocation or claim
that a previously implemented generic provider has stopped working. Keyring reliability concerns and
possible expiry failures motivate the design; they do not establish a new uv correctness regression
here. The consolidation suggestion in astral-sh/uv#19025 comes from a contributor, not a maintainer
decision approving a particular design.

## Report breakdown

- **Credential acquisition:** store an executable and argument list for a service and invoke it when
  authentication is needed. Allow an optional username override.
- **Expiry and caching:** support short-lived tokens; consider no caching, a brief cache, or an
  explicit lifetime. The reporter leaves the exact policy open.
- **Configuration and execution boundary:** require explicit local registration through `uv auth`;
  prohibit executable configuration in `pyproject.toml` and `uv.toml`. Proposed safeguards include
  bounded output, no token logging, protected storage, and fixed arguments without dynamic context.
- **Setup motivation:** avoid wrappers that vary by shell or operating system and the installation,
  backend-selection, and bootstrapping costs of keyring plugins.
- **Separate extension:** Bearer authentication is discussed as a possible future capability; the
  proposed provider initially works with Basic authentication.
- **Environment and identifiers:** enterprise/private indexes with an existing token-vending CLI;
  macOS is illustrative, not an exclusive trigger. Relevant identifiers include
  `--token-provider`, `--username`, `UV_INDEX_<NAME>_PASSWORD`, and `native-auth`.
  No exact error or uv release regression is reported.

## Related

- astral-sh/uv#7660 — Allow command for `tool.uv.index-url` and `tool.uv.pip.index-url` (issue, open). Canonical discussion for obtaining dynamic index credentials by invoking a command without keyring setup. The reporter's September 25 comment already proposes local-only token-helper configuration through uv auth, excluding project configuration. It remains labeled needs-design and needs-decision.
- astral-sh/uv#19025 — pluggable credentials providers (issue, open). Requests arbitrary credential commands, optional token-expiry caching, and avoiding keyring bootstrapping. A contributor points to astral-sh/uv#7660 for consolidation and raises project-configured command execution concerns. The new proposal refines the configuration boundary for the same capability.
- astral-sh/uv#19611 — Add Google Artifact Registry credential retrieval (pull request, open, draft). Draft implementation retrieves Google credentials through Application Default Credentials or gcloud on Unix, with expiry-aware caching and no required keyring plugin. Relevant to dynamic credentials, but limited to Google Artifact Registry rather than a configurable external token provider.
- astral-sh/uv#18907 — Rewrite the native authentication storage scheme (pull request, open, draft). Reporter-cited draft rewrites native credential storage and matching, including structured credentials and lookup without a username. It does not implement external token commands or establish that such providers must depend on this rewrite.
- astral-sh/uv#18541 — Support Bearer Authentication for Package Registry (issue, open). Tracks passing OAuth tokens from a custom keyring backend as Bearer authentication. This covers the report's optional authentication-scheme extension; invoking a token provider that supplies a Basic-auth password is a separate capability.

## Supporting evidence

- In astral-sh/uv#7660, the October 3, 2024 clarification requests dynamic credentials without
  installing a keyring backend, citing `credential_process`. The September 25, 2026 comment from
  this reporter explicitly requests a locally stored `uv auth` token helper. The issue remains
  open with `needs-design` and `needs-decision`.
- astral-sh/uv#19025 proposes both a credential-producing command and
  `credentials-expire-seconds`. Its discussion points back to astral-sh/uv#7660 and discusses
  the risk of executing commands from repository configuration.
- At checkout commit `01b62808962d7abfe2d10f43d652f357d8038202`,
  `crates/uv-cli/src/lib.rs:6401` defines `AuthLoginArgs` with username/password/token inputs
  and no token-provider command option. `crates/uv-auth/src/keyring.rs:272` invokes the fixed
  `keyring` executable; retrieved values are mapped to Basic credentials.
- `docs/concepts/authentication/http.md:51` documents the subprocess keyring interface.
  Lines 62–66 describe caching for the duration of a uv invocation, without carrying that
  authentication cache across invocations. This is distinct from persistent credential storage.
  The documentation does not establish a generic expiry/refresh protocol for external commands.
- `docs/guides/integration/aws.md:57` documents the existing CodeArtifact keyring integration,
  including short-lived-token refresh by the plugin and the requirement to install the backend
  before accessing the private index. In astral-sh/uv#16684, a maintainer explains that uv must
  resolve dependencies before constructing the environment containing a project-local keyring.
- `docs/concepts/authentication/cli.md` describes `uv auth helper` as exposing uv credentials to
  external tools through the Bazel protocol. That is the opposite direction from consuming an
  arbitrary token-producing command.
- The inspected diff of draft astral-sh/uv#19611 adds Google-specific credential discovery,
  a short cache with expiry handling, and a Unix `gcloud` fallback. It supplies useful adjacent
  implementation context without providing a generic command-registration interface.
- Draft astral-sh/uv#18907 and its review discussion concern native-store serialization,
  credential matching, locking, and migration. No reviewed evidence establishes it as a required
  dependency for the proposed provider.
- astral-sh/uv#18541 concerns Bearer transport for keyring-supplied OAuth tokens and references
  the broader header-auth discussion in astral-sh/uv#1369. Neither is the canonical discussion
  for obtaining tokens through a configurable command.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues using token-provider, credentials-provider, credentials-expire-seconds, short-lived, credential provider, pluggable, credential_process, keyring command, token refresh/expired/cache, arbitrary command execution, bootstrapping, and Bearer. Searched open, closed, and merged PRs for auth, credential/provider, keyring, token-provider, and token refresh. PR keyword searches returned no matches; supplemented them with title filtering of the latest 1,000 all-state PRs and up to 1,000 open PRs, plus direct reads of referenced PRs. Inspected candidate bodies, comments, reviews, relevant diffs, and current authentication documentation/source. Ruled out astral-sh/uv#3009 and merged astral-sh/uv#3010: they fix package-cache invalidation when credentials change, not token acquisition or expiry. Also distinguished astral-sh/uv#12755 (keyring executable selection), astral-sh/uv#8523 and merged astral-sh/uv#21275 and astral-sh/uv#18246 (tool-upgrade credential handling), and astral-sh/uv#19136 (built-in OAuth device flow).

Additional distinctions from inspected candidates:

- astral-sh/uv#12755 requests choosing the existing keyring executable, retaining its protocol.
- astral-sh/uv#3009 and merged astral-sh/uv#3010 concern package-cache URL hashing after token
  rotation. They do not implement token refresh or provider execution.
- astral-sh/uv#8523 concerns old credentials in tool receipts. Merged astral-sh/uv#21275 reuses
  configured credentials during tool upgrades; merged astral-sh/uv#18246 changes authentication
  policy when saving receipts. Neither implements the proposed provider.
- astral-sh/uv#19136 proposes a built-in OAuth device flow rather than an external command.
- Closed astral-sh/uv#18618 concerns rejecting existing Bearer credentials under
  `authenticate = "always"`, not a missing token-provider interface.
- Closed astral-sh/uv#15633 asks about running authentication before commands; maintainer replies
  recommend the existing CodeArtifact keyring plugin.

The PR title scans are bounded to the stated limits; empty keyword-search results are not evidence
that no other related PR exists. The duplicate classification rests on the directly inspected open
discussions, rather than absence of search results.
