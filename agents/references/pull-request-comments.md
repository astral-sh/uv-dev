# Pull request automation comments

The `pull-request-labels.yml` workflow attempts to reserve a comment within the first three comments
on newly opened and reopened `uv-dev` pull requests. The reservation runs independently of label
analysis and uses `astral-automations-bot`. It reuses a matching comment in those positions or
attempts to create one if fewer than three comments are present when checked.

To find the reservation, run:

```sh
uv run --script scripts/pull-request-comment.py find --pull-request "$PULL_REQUEST_NUMBER"
```

The command reads only the first three issue comments in one request and prints the first matching
comment's ID if both its bot identity and `<!-- uv-automation-comment -->` prefix match. It prints
nothing when there is no reservation in those positions. Other commenters can arrive before the
reservation, and older pull requests may already have comments.

Keep the marker followed by a newline at the start when updating the comment. Writers must serialize
their read-modify-write operations with the job concurrency group
`pull-request-automation-comment-${{ inputs.pull_request }}` and `queue: max`. Use the same pull
request number when the workflow names its input differently. Update only the report owned by the
writer, leaving other automation updates intact.
