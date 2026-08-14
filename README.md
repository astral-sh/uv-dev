# Remove receipt in uninstallation instructions

Issue: astral-sh/uv#21127

Classification: duplicate

## Summary

The reporter installed uv on Windows with the standalone installer, followed the documented
uninstallation steps, and then installed uv with WinGet. Running `uv self update` from the WinGet
installation loaded the old `%LOCALAPPDATA%/uv/uv-receipt.json` and reported that the current
executable did not match the standalone install location, suggesting that multiple copies might be
installed. Removing the stale receipt cleared the condition, so the reporter asks whether the
uninstallation documentation should include receipt cleanup.

astral-sh/uv#12647 is an exact open match: it asks why the same receipt is not removed by the same
uninstallation instructions. A maintainer said the project could recommend deleting it, while
noting that cargo-dist managed where the receipt was stored. The new report adds a concrete Windows
and WinGet consequence, but the documentation decision can be centralized in that existing issue.

The current documentation removes stored cache, Python, and tool data optionally and then removes
the binaries, but does not mention the standalone installer receipt. The current self-update source
loads an available receipt and checks whether it belongs to the running executable; on a mismatch,
it emits the multiple-copies diagnostic quoted by the reporter. This establishes the interaction
without requiring an unconfirmed additional root cause.

## Draft response

Thanks for documenting the concrete WinGet migration case. The missing `uv-receipt.json` cleanup
guidance is already tracked in astral-sh/uv#12647, where maintainers noted that recommending
deletion is possible even though the receipt is managed by cargo-dist. The current uninstallation
page does not remove the receipt, and the self-update path checks a loaded receipt against the
running executable, which is consistent with the diagnostic you saw. Let’s centralize the
documentation decision in astral-sh/uv#12647; closing this as a duplicate.

## Classification

This is a duplicate because open issue astral-sh/uv#12647 already tracks the same requested change:
documenting deletion of `uv-receipt.json` on the uv uninstallation page. The new Windows-to-WinGet
sequence is useful supporting evidence, but it does not require a separate tracking issue. The
duplicate classification takes precedence over classifying the request as an enhancement or the
misleading diagnostic as a bug.

## Related

- astral-sh/uv#12647 — “why there is no information to delete `uv-receipt.json` when uninstalling
  `uv`” (open issue). This is the canonical exact match: it names the same file and documentation
  page. Maintainer comments say deletion could be recommended, although the receipt location was
  managed by cargo-dist.
- astral-sh/uv#12686 — “`uv self update` fails in MSYS2 environment” (open issue). This contains
  the same Windows executable-versus-receipt diagnostic and confirms the receipt lookup behavior,
  but the trigger is different: MSYS2 produces a receipt with the wrong location/path
  representation, rather than leaving a standalone receipt behind during a migration to WinGet.
- astral-sh/uv#21129 — “Document removing uv-receipt.json after uninstall on Windows” (closed pull
  request). This directly proposed the documentation change in response to astral-sh/uv#21127, but
  it was closed without merging and does not replace the older canonical issue.

## Search scope

The search covered open and closed issues and open, closed, and merged pull requests. Literal terms
included `uv-receipt.json`, fragments of “The current executable is at” and “standalone installer
was used,” `receipt` with `uninstall` or `uninstallation`, and `self update` with `multiple copies`
or `winget`. Conceptual searches covered standalone-installer metadata, switching package managers,
receipt cleanup, and a self-uninstall command. Fix-oriented searches checked documentation changes,
self-uninstall implementations, and pull requests referencing astral-sh/uv#12647 or
astral-sh/uv#21127.

The broader astral-sh/uv#9871 and astral-sh/uv#11613 self-uninstall work was inspected but does not
specifically track this receipt-cleanup documentation gap. The historical astral-sh/uv#1696 and
astral-sh/uv#9938 work added and reorganized general uninstallation guidance but did not cover the
receipt. astral-sh/uv#2428 concerned a Homebrew uninstall and an unrelated Rust build error.
