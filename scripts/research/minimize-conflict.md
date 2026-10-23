# Bounded root conflict minimization

```sh
python scripts/research/minimize_conflict.py --uv /absolute/path/to/uv --work-dir "$PWD/work" --demo
python scripts/research/minimize_conflict.py --uv /absolute/path/to/uv --work-dir "$PWD/work" --demo --max-solves 2
python scripts/research/minimize_conflict.py --uv /absolute/path/to/uv --work-dir "$PWD/work" --demo multiple
```

The first command isolates the conflicting roots `left==1` and `right==1` from two unrelated roots.
The second exhausts its budget and reports the smallest subset already observed to be unsatisfiable,
without claiming minimality. A timeout before any conflict is established yields `null` for the
conflict. JSON includes solve count, cache hits, elapsed time, and a wheel fingerprint.

The multiple-core demo contains two independent conflicting pairs. The command returns one
subset-minimal core; changing root order can select the other pair. It does not enumerate every
core. Use `--background background.in` to include fixed requirements in every solve while minimizing
only the supplied roots. Background uses the same restricted grammar as roots. Its consistency is
checked first: `background_conflict` with an empty conflict list means root removal cannot repair
it, while `null` means no conflict was established within the budget. A background conflict is not
reported as a minimized root conflict. Background consistency consumes an actual solve from the same
budget.

Use `--requirements roots.in --wheelhouse /path/to/wheels` instead of `--demo` for a local corpus.
The script snapshots wheels into its work directory, rejects direct-reference dependencies, and
disables network access, indexes, source builds, and persistent caches. Roots support package names
and version specifiers; options, markers, extras, and direct references are deliberately excluded.
Resolution is fixed to Python 3.12 on Linux with `--prerelease allow`. Keeping candidate eligibility
independent of explicit prerelease roots is required for the monotone consistency predicate used by
QuickXplain. Recognized malformed-artifact diagnostics and unrecognized uv failures abort the
experiment rather than count as conflicts.

The divide-and-conquer procedure follows
[QuickXplain](https://link.springer.com/article/10.1007/s10462-022-10149-w). It memoizes subsets and
explicitly verifies unsatisfiability and every single-root deletion before reporting subset
minimality. This is not a minimum-cardinality guarantee, and the result can depend on root order.
The experiment minimizes user roots over immutable metadata; it does not minimize PubGrub's internal
proof, modify uv's diagnostic rendering, or preserve user root priorities.

uv may describe malformed candidate metadata or wheel structure inside a no-solution diagnostic. The
wrapper rejects recognized invalid or inconsistent metadata, parse failures, and invalid package
format diagnostics before treating no-solution output as a consistency answer. This classification
depends on uv's textual diagnostics; it is not a structured proof API.

Run with `--demo invalid-metadata` to exercise a malformed `Requires-Dist` control. It must exit
with an error and emit no conflict JSON, even though uv wraps that metadata failure in a no-solution
diagnostic.

Run with `--demo invalid-structure` to exercise a wheel whose `.dist-info` directory names a
different package. It contains exactly one readable `METADATA` file, so the corpus preflight accepts
it, but uv rejects the wheel structure. The wrapper must exit with an error and emit no conflict
JSON.
