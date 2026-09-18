# Audit reports

!!! note

    `uv audit` and its machine-readable output are in preview. The command and report schemas may
    change without warning.

`uv audit` can report known vulnerabilities and adverse project statuses for the selected project or
script dependencies as JSON:

```console
$ uv audit --output-format json --preview-features audit,json-output
```

The [JSON schema](audit.schema.json) describes the current report. It contains the preview schema
version, summary counts, vulnerability findings, and adverse project statuses. Advisory details that
are unavailable from the source are represented by `null` where the schema permits it.

With `--output-format jsonl`, the [JSONL record schema](audit-jsonl.schema.json) describes both
progress records and the final report, which has `"type": "result"`. Pass
`--preview-features audit,jsonl` to enable both preview features. Use `--no-progress` or `--quiet`
to emit only the result; repeated `--quiet` suppresses it as well.

Consumers must also check the process exit status. A completed audit can return a valid report with
a nonzero status when it finds vulnerabilities. A setup failure can terminate without a result.

The schemas are generated from uv's report serialization types. They describe the current preview
format; their availability does not make that format stable.
