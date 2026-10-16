# Bounded root conflict minimization

```sh
python scripts/research/minimize_conflict.py --uv /absolute/path/to/uv --work-dir "$PWD/work" --demo
python scripts/research/minimize_conflict.py --uv /absolute/path/to/uv --work-dir "$PWD/work" --demo --max-solves 2
```

The first command isolates the conflicting roots `left==1` and `right==1` from two unrelated roots.
The second exhausts its budget and reports the smallest subset already observed to be unsatisfiable,
without claiming minimality. A timeout before any conflict is established yields `null` for the
conflict. JSON includes solve count, cache hits, elapsed time, and a wheel fingerprint.

Use `--requirements roots.in --wheelhouse /path/to/wheels` instead of `--demo` for a local corpus.
The script snapshots wheels into its work directory, rejects direct-reference dependencies, and
disables network access, indexes, source builds, and persistent caches. Roots support package names
and version specifiers; options, markers, extras, and direct references are deliberately excluded.
Resolution is fixed to Python 3.12 on Linux with `--prerelease allow`. Keeping candidate eligibility
independent of explicit prerelease roots is required for the monotone consistency predicate used by
QuickXplain. Metadata errors and unrecognized uv failures abort the experiment rather than count as
conflicts.

The divide-and-conquer procedure follows
[QuickXplain](https://link.springer.com/article/10.1007/s10462-022-10149-w). It memoizes subsets and
explicitly verifies unsatisfiability and every single-root deletion before reporting subset
minimality. This is not a minimum-cardinality guarantee, and the result can depend on root order.
The experiment minimizes user roots over immutable metadata; it does not minimize PubGrub's internal
proof, modify uv's diagnostic rendering, or preserve user root priorities.
