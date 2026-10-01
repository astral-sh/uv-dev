#!/usr/bin/env bash
set -euo pipefail

memory_gib="$1"
results="$2"
diagnostic_script="$3"
build_count="${4:-3}"
cpu_count="${5:-16}"
case "$memory_gib" in 32|64) ;; *) exit 2 ;; esac
case "$build_count" in 1|2|3|4) ;; *) exit 2 ;; esac
case "$cpu_count" in 16|32) ;; *) exit 2 ;; esac
cpuset="0-$((cpu_count - 1))"
mkdir -p "$results"
containers=()
for ((number=1; number<=build_count; number++)); do
    containers+=("uv-ci-2124-$number")
done
monitor_pid=""

collect() {
    exit_status="${1:-$?}"
    trap - EXIT
    touch "$results/stop"
    if test -n "$monitor_pid"; then
        wait "$monitor_pid" || true
    fi
    for container in "${containers[@]}"; do
        if docker inspect "$container" > /dev/null 2>&1; then
            docker stop --time 10 "$container" > /dev/null 2>&1 || true
            docker logs --timestamps "$container" > "$results/$container.log" 2>&1 || true
            docker inspect --format '{{.State.ExitCode}}' "$container" > "$results/$container.exit" || true
            docker cp "$container:/root/.ci-2124/results" "$results/$container" || true
            docker rm "$container" > /dev/null || true
        fi
    done
    uv run --no-config --no-project --python 3.11 "$diagnostic_script" snapshot > "$results/host-after.json" || true
    sudo -n dmesg -T | tee "$results/kernel-after.log" > /dev/null || true
    sudo systemctl stop uvdiag2124-anchor.service uvdiag2124.slice || true
    exit "$exit_status"
}
trap 'collect $?' EXIT

test "$(docker info --format '{{.CgroupDriver}}')" = systemd
sudo systemd-run --unit=uvdiag2124-anchor --slice=uvdiag2124.slice \
    --property=Type=oneshot --property=RemainAfterExit=yes /usr/bin/true
sudo systemctl set-property --runtime uvdiag2124.slice \
    "MemoryMax=${memory_gib}G" MemorySwapMax=0 "AllowedCPUs=$cpuset"
control_group="$(systemctl show uvdiag2124.slice --property=ControlGroup --value)"
test -n "$control_group"
export UV_DIAGNOSTIC_CGROUP="/sys/fs/cgroup$control_group"
test "$(cat "$UV_DIAGNOSTIC_CGROUP/memory.max")" = "$((memory_gib * 1024 * 1024 * 1024))"
test "$(cat "$UV_DIAGNOSTIC_CGROUP/memory.swap.max")" = 0
test "$(cat "$UV_DIAGNOSTIC_CGROUP/cpuset.cpus.effective")" = "$cpuset"

{ uname -sr; lscpu; free -b; swapon --show; docker image inspect --format '{{.Id}}' uv-ci-2124:toolchain; } > "$results/machine.txt"
sudo -n dmesg -T | tee "$results/kernel-before.log" > /dev/null || true
uv run --no-config --no-project --python 3.11 "$diagnostic_script" snapshot > "$results/host-before.json"
uv run --no-config --no-project --python 3.11 "$diagnostic_script" monitor "$results/host-memory.jsonl" "$results/stop" < /dev/null > "$results/monitor.log" 2>&1 &
monitor_pid="$!"

for container in "${containers[@]}"; do
    docker create --name "$container" --cpuset-cpus="$cpuset" \
        --cgroup-parent=uvdiag2124.slice -e TARGETPLATFORM=linux/arm64 \
        uv-ci-2124:toolchain /root/.ci-2124/build.sh > /dev/null
done
for container in "${containers[@]}"; do
    docker start "$container" > /dev/null
    container_pid="$(docker inspect --format '{{.State.Pid}}' "$container")"
    # Check the actual hierarchy before starting the next memory-intensive build.
    sudo cat "/proc/$container_pid/cgroup" | grep -F "0::$control_group/" > "$results/$container.cgroup"
done

failed=0
for container in "${containers[@]}"; do
    result="$(docker wait "$container")"
    printf '%s %s\n' "$container" "$result" | tee -a "$results/exits.txt"
    if test "$result" != 0; then
        failed=1
    fi
done
collect "$failed"
