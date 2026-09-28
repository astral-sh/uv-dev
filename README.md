# inode count for `uv cache size`

Issue: astral-sh/uv#22053

Classification: enhancement

## Summary

The issue requests that `uv cache size` report how many inodes the cache consumes, motivated by
filesystems on HPC clusters that enforce strict inode quotas. The reporter suggests a mode such as
`uv cache size --inodes` and raises presentation questions including flag naming, abbreviated versus
raw counts, and whether byte size and inode count should be displayed together.

No exact duplicate was found. The closest open discussion is astral-sh/uv#1655, which proposes
broader cache inspection modeled on `pip cache info` and explicitly includes file counts. That issue
does not clearly cover a total inode-consumption metric, including the treatment of directories,
symlinks, and hard links. The merged pull requests astral-sh/uv#16032 and astral-sh/uv#20992 define
the command and its current output-format behavior.

Source inspection confirms that `uv cache size` currently uses `diskus::DiskUsage` to traverse the
cache and emits only a total byte count. There is no inode-count output path today.

## Draft response

The inode-quota use case is clear, and this would be a new capability for the preview `uv cache
size` command. The command currently uses diskus to calculate only total bytes. astral-sh/uv#1655
is related because its broader cache-info proposal includes file counts, but it does not specifically
cover total inode consumption.

Before implementation, we should define the metric precisely: whether to count directory entries or
distinct inode objects, and whether directories and symlinks are included. That distinction matters
for hard-linked cache contents and portability. Once those semantics are agreed, we can decide how
the new metric should fit the automatic, human, and machine output modes added in
astral-sh/uv#20992.

## Classification

This is an enhancement. Existing behavior reports byte size as designed; the issue asks for an
additional cache statistic and related CLI presentation. The current source confirms the requested
metric is absent rather than malfunctioning.

astral-sh/uv#1655 overlaps because its proposed cache information includes file counts, but its scope
is categorized cache information and listing. A filesystem inode metric has distinct semantics,
especially for directories, symlinks, and hard-linked files, so the new request should not be marked
as a duplicate.

## Related

- astral-sh/uv#1655 — Open issue, “Add `pip cache info` and `pip cache list`.” This is the closest
  existing request because its example explicitly reports numbers of cached files. It is broader
  than the new issue and does not specifically track total inode consumption.
- astral-sh/uv#16032 — Merged pull request, “Add a `uv cache size` command.” It introduced the
  command being extended and implemented total-byte output with a human-readable option.
- astral-sh/uv#20992 — Merged pull request, “Add `--output-format` to `uv cache size`.” It established
  the current automatic, human, and machine presentation modes, which are directly relevant to the
  proposed count formatting.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `inode`, `uv cache size`, `cache-size`, and `diskus`. Conceptual queries covered file count,
number of files, cache information/statistics/inspection, disk usage, quotas, HPC, cache limits,
hard links, and link counts. The original cache-size issue and implementation, the later
output-format issue and implementation, the broader cache-info request and its originating
discussion, and cache counting/management reports were inspected along with their comments and
references.

astral-sh/uv#7642 was a plausible lead because it documents HPC inode quotas, but its requested
changes concern virtual-environment placement and architecture-sensitive wheel caching, not cache
inode reporting. Issues about misleading freed-byte totals and hard-link-aware pruning were also
ruled out because they concern cache cleanup semantics rather than exposing an inode count.
