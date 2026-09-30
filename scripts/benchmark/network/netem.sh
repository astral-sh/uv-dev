#!/usr/bin/env bash
set -euo pipefail

# The private namespace has only loopback. No host interface or route is changed.
if [[ ${1:-} != --inside ]]; then
  if [[ $# -lt 6 ]]; then
    echo "Usage: $0 RTT_MS RATE_MBIT LOSS_PERCENT SEED_OR_NONE -- COMMAND..." >&2
    exit 2
  fi
  if [[ $(id -u) == 0 ]]; then
    exec unshare --net -- "$0" --inside "$@"
  fi
  exec unshare --user --map-root-user --net -- "$0" --inside "$@"
fi
shift
rtt_ms=$1
rate_mbit=$2
loss_percent=$3
seed=$4
shift 4
[[ $1 == -- ]] || { echo "Expected -- before command" >&2; exit 2; }
shift
for value in "$rtt_ms" "$rate_mbit" "$loss_percent"; do
  [[ $value =~ ^[0-9]+([.][0-9]+)?$ ]] || { echo "Invalid numeric argument" >&2; exit 2; }
done
seed_args=()
seed_json=null
if [[ $seed != none ]]; then
  [[ $seed =~ ^[0-9]+$ ]] || { echo "Invalid seed" >&2; exit 2; }
  seed_args=(seed "$seed")
  seed_json=$seed
fi
printf -v UV_BENCH_NETEM '{"rtt_ms":%s,"rate_mbit":%s,"loss_percent":%s,"seed":%s}' "$rtt_ms" "$rate_mbit" "$loss_percent" "$seed_json"
export UV_BENCH_NETEM
half_rtt=$(awk -v rtt="$rtt_ms" 'BEGIN { printf "%.3f", rtt / 2 }')
ip link set lo up
tc qdisc add dev lo root netem delay "${half_rtt}ms" rate "${rate_mbit}mbit" loss random "${loss_percent}%" "${seed_args[@]}"
trap 'tc -s qdisc show dev lo >&2' EXIT
if [[ $(id -u) == 0 && -n ${SUDO_UID:-} && -n ${SUDO_GID:-} ]]; then
  setpriv --reuid "$SUDO_UID" --regid "$SUDO_GID" --init-groups -- "$@"
else
  "$@"
fi
