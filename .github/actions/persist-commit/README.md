# Commit artifacts

Use `persist-commit` to transfer an existing commit from an unprivileged job to a separate consumer.
It uploads a Git bundle containing the history after `base-sha` and returns its immutable
`artifact-id` and exact `head-sha`. The base must be a strict ancestor of the commit. The action
does not create a commit from working-tree changes.

Pass both outputs to `load-commit`, along with the trusted base SHA. The consumer must already have
the base and the bundle's prerequisite history, normally through `actions/checkout` with
`fetch-depth: 0`. `load-commit` accepts one artifact from the current workflow run, requires exactly
the expected bundle ref and SHA, imports its objects, and checks ancestry. It does not check out the
commit or change the consumer's refs or `FETCH_HEAD`.

```yaml
- uses: ./.github/actions/persist-commit
  id: persist
  with:
    name: prepared-commit-${{ github.run_id }}-${{ github.run_attempt }}
    base-sha: ${{ github.sha }}
    head-sha: ${{ steps.prepare.outputs.head-sha }}
```

Expose `steps.persist.outputs.artifact-id` and `steps.persist.outputs.head-sha` as job outputs, then
load them in a fresh job:

```yaml
- uses: ./.github/actions/load-commit
  id: load
  with:
    artifact-id: ${{ needs.prepare.outputs.artifact-id }}
    base-sha: ${{ github.sha }}
    head-sha: ${{ needs.prepare.outputs.head-sha }}
```

Run both local actions from the trusted workflow checkout. If preparation switches to another
revision, capture the result SHA and restore the trusted revision before invoking `persist-commit`.
Consumers must still validate their own policy, such as exact parents, author, commit message,
allowed paths, file modes, and whether the target branch has changed. Importing a valid bundle is
not authorization to publish it. Request write credentials only after those checks pass, and do not
execute files from the imported commit in a privileged job.

Run the transport tests with `python3 scripts/test_commit_artifact.py`.
