#!/bin/sh
# Run a command with a private writable view of a relocatable Linux venv.
# Usage: sh overlay_environment.sh SEED NEW_JOB COMMAND [ARG ...]
#
# Prepare the seed with `uv venv --relocatable`, then install the desired
# packages with `--link-mode copy` and without editables. Running requires
# permission to create a mount namespace and mount OverlayFS. Changes made
# through the mounted environment go to the job's upper layer. Remove the job
# directory after its processes exit when the upper layer is no longer needed.
set -eu
if [ "$#" -lt 3 ]; then
    echo 'usage: overlay_environment.sh SEED NEW_JOB COMMAND [ARG ...]' >&2
    exit 2
fi
seed=$(CDPATH='' cd -P -- "$1" && pwd)
parent=$(CDPATH='' cd -P -- "$(dirname -- "$2")" && pwd)
job="$parent/$(basename -- "$2")"
shift 2
for directory in "$seed" "$job"; do
    case "$directory" in
        *:*|*,*|*\\*) echo 'mount paths must not contain colons, commas, or backslashes' >&2; exit 2 ;;
    esac
done
case "$job/" in
    "$seed/"*) echo 'choose a job directory outside the seed' >&2; exit 2 ;;
esac
if ! grep -Fxq 'relocatable = true' "$seed/pyvenv.cfg"; then
    echo 'create the seed with uv venv --relocatable' >&2
    exit 2
fi
mkdir -m 700 -- "$job"
mkdir -- "$job/upper" "$job/work" "$job/env"

# The mount disappears with the job's namespace. The parent namespace sees an
# empty mount point even if the command exits with an error.
# The inner shell expands its positional arguments after entering the namespace.
# shellcheck disable=SC2016
exec unshare --mount --propagation private sh -ec '
    seed=$1 job=$2
    shift 2
    mount -t overlay overlay -o "lowerdir=$seed,upperdir=$job/upper,workdir=$job/work" "$job/env"
    VIRTUAL_ENV=$job/env
    PATH=$VIRTUAL_ENV/bin:$PATH
    export VIRTUAL_ENV PATH
    exec "$@"
' sh "$seed" "$job" "$@"
