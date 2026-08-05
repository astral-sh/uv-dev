# uv tries to open recently written trampoline executable for exclusive access, flagged by AV

Issue: astral-sh/uv#20955
Classification: bug

## Summary

astral-sh/uv#20567 is the closest historical report, astral-sh/uv#20792 is the active umbrella tracker, and astral-sh/uv#19873 records the same error with less diagnosis. astral-sh/uv#15068 introduced the relevant PE-resource implementation; no targeted fix PR was found.

## Classification

The observable behavior is a correctness failure: trampoline creation can fail during normal uv operations on affected Windows systems. Repository source confirms that uv writes a temporary executable and then performs a BeginUpdateResourceW/UpdateResourceW/EndUpdateResourceW transaction, while astral-sh/uv#20567 experimentally isolates the failure to EndUpdateResourceW under Bitdefender. The stronger claims that EndUpdateResourceW reopens without sharing and that Bitdefender actively injects the exception remain reporter hypotheses, not repository-confirmed root causes. This is not classified as a duplicate because the exact prior issue was closed into a broad tracker whose maintainer subsequently invited separate reports for newly distinguished failure modes.

## Related

- https://github.com/astral-sh/uv/issues/20567 (closed issue): uv venv: Windows PE resource error due to Bitdefender Incident Sensor
  Closest prior report: astral-sh/uv#20567 reproduces the same Windows trampoline PE-resource failure with Bitdefender, identifies EndUpdateResourceW as the failing call, and reports ERROR_OPEN_FAILED. It was closed in favor of astral-sh/uv#20792.
- https://github.com/astral-sh/uv/issues/20792 (open issue): Windows antivirus/EDR issues
  The open AV/EDR tracker contains the reporter’s exact mechanism hypothesis. However, after that comment a maintainer explicitly requested separate issues for new failure modes, so it is related context rather than a clear duplicate target.
- https://github.com/astral-sh/uv/issues/19873 (closed issue): failt to create venv in windows
  Reports the identical uv-trampoline PE-update error and ERROR_OPEN_FAILED on Windows 11; disabling an unspecified security tool resolved it. It lacks the Bitdefender-specific diagnosis of astral-sh/uv#20567.
- https://github.com/astral-sh/uv/pull/15068 (merged pull request): Use `.rcdata` to store trampoline type + path to python binary
  Introduced the current PE-resource-based trampoline metadata implementation. Its discussion notes the temporary-file write/read/delete round trip, but does not confirm the reported exclusive-open or AV exception-injection mechanism.
