#!/bin/sh
# Bound each corpus resolution, including descendants started by build backends.
exec timeout --kill-after=10s 600s /bin/uv-bench "$@"
