# Adding AWS SSO support for S3 Endpoints

Issue: astral-sh/uv#21976

Classification: enhancement

## Summary

The report asks uv's experimental S3 request-signing support to select AWS profiles through
`AWS_PROFILE` or `AWS_DEFAULT_PROFILE`, obtain and refresh credentials for AWS IAM Identity Center
(SSO), and allow an index to opt into S3 signing with a setting such as `signing = "s3"` instead of
relying on `UV_S3_ENDPOINT_URL`.

Part of this behavior already exists. The S3 signer added by astral-sh/uv#15925 uses reqsign's
default AWS credential chain. In the current checkout, that chain reads `AWS_PROFILE`,
`AWS_CONFIG_FILE`, and `AWS_SHARED_CREDENTIALS_FILE`; it also has an SSO provider that can exchange
a valid cached SSO access token for temporary role credentials. The signer caches those role
credentials and reloads the provider when they expire.

The current dependency does not honor `AWS_DEFAULT_PROFILE`, follow a modern `sso_session` section,
or refresh/initiate login when the cached SSO access token itself is expired. uv also still chooses
S3 signing globally by matching `UV_S3_ENDPOINT_URL`, not through per-index configuration. No
existing uv issue or pull request was found that tracks that same remaining scope.

## Draft response

AWS_PROFILE is already supported for S3 endpoints through the default AWS credential chain added
in astral-sh/uv#15925. The current chain can also exchange a valid cached SSO login token for role
credentials and reload those role credentials when they expire. It does not currently honor
AWS_DEFAULT_PROFILE, follow an sso_session section, or refresh/initiate login when the cached SSO
access token itself has expired. S3 signing is also selected globally with UV_S3_ENDPOINT_URL rather
than with an index-level signing setting.

Those remaining pieces would be enhancements and need agreement on the configuration and
authentication scope before implementation. Could you confirm whether the needed refresh is for an
expired SSO access token, rather than expiring role credentials, and whether the profile uses inline
SSO fields or an sso_session section? That will let us separate the credential-chain gap from the
proposed per-index API.

## Classification

This is an enhancement. The report proposes additions to an experimental feature rather than
demonstrating incorrect established behavior or a regression. Repository source and
astral-sh/uv#15925 confirm that `AWS_PROFILE`, AWS config and credential files, and valid cached SSO
tokens already participate in the current credential chain. The unimplemented portions are
`AWS_DEFAULT_PROFILE`, modern or expired SSO-session handling, and an index-level signing setting.
No open issue or pull request already tracks that combination, so the report is not a duplicate.

## Related

- astral-sh/uv#15925, **Add S3 request signing** (merged pull request): This is the foundational
  implementation for the reported subsystem. It added endpoint-based S3 SigV4 signing through
  reqsign's default AWS credential chain. It establishes that `AWS_PROFILE` and valid cached SSO
  tokens are already recognized, but it does not provide `AWS_DEFAULT_PROFILE`, expired SSO
  login-token refresh, or per-index signing configuration.
- astral-sh/uv#13608, **Recognize an S3 store as valid for packages** (open issue): This is an
  adjacent request about recognizing S3 or MinIO storage as a package repository. It does not track
  AWS profile selection, SSO token refresh, or configuring signing for an index, so it is not a
  duplicate.

## Search evidence

Literal searches covered `AWS_PROFILE`, `AWS_DEFAULT_PROFILE`, `AWS SSO`, `SSO token`, `AWS
profile`, `Identity Center`, and `UV_S3_ENDPOINT_URL`. Conceptual searches covered S3
credentials/authentication, AWS default credential and profile chains, SigV4, signed private and
object-storage package indexes, temporary-credential refresh, and per-index cloud signing.
Fix-oriented searches covered closed issues and merged pull requests for S3, AWS, reqsign, and
cloud signing.

The discussion and review threads for astral-sh/uv#15925 were inspected, as were the signer-lifetime
change in astral-sh/uv#17092 and the current reqsign update in astral-sh/uv#20892. The current uv and
reqsign sources confirm the credential-chain behavior described above. astral-sh/uv#19025 was ruled
out as a generic executable credential-provider request centered on CodeArtifact.
astral-sh/uv#14067 was ruled out because it concerns CloudFront 403/index fallback behavior rather
than AWS credential discovery.
