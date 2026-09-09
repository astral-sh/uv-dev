"""Checks for race-prone proc sampling and optional performance counters."""

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from monitor import Monitor, parse_stat, perf_command, perf_supported, process_tree


def stat(pid, parent, name="test process) worker"):
    # Fields 3 through 52 from proc_pid_stat(5).
    fields = ["0"] * 50
    fields[0], fields[1] = "S", str(parent)
    for index, value in {
        7: 123,
        9: 4,
        11: 50,
        12: 25,
        17: 1,
        19: 321,
        21: 500,
        36: 2,
        39: 7,
    }.items():
        fields[index] = str(value)
    return f"{pid} ({name}) " + " ".join(fields)


class MonitorTests(unittest.TestCase):
    @unittest.skipUnless(Path("/proc/self/stat").exists(), "Linux procfs required")
    def test_live_linux_process_snapshot(self):
        with subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(2)"]
        ) as child:
            try:
                tree = process_tree(child.pid)
                self.assertEqual([row["pid"] for row in tree["processes"]], [child.pid])
                self.assertGreaterEqual(len(tree["processes"][0]["tasks"]), 1)
                self.assertIsInstance(tree["processes"][0]["io"], str)
            finally:
                child.terminate()
                child.wait(timeout=5)

    def test_stat_names_and_counter_positions(self):
        row = parse_stat(stat(10, 1))
        self.assertEqual(
            row,
            {
                "pid": 10,
                "comm": "test process) worker",
                "state": "S",
                "ppid": 1,
                "minor_faults": 123,
                "major_faults": 4,
                "user_ticks": 50,
                "system_ticks": 25,
                "threads": 1,
                "start_ticks": 321,
                "rss_pages": 500,
                "cpu": 2,
                "block_io_delay_ticks": 7,
            },
        )

    def test_tree_keeps_descendants_and_separates_vm_background(self):
        with tempfile.TemporaryDirectory() as directory:
            proc = Path(directory)
            for pid, parent in ((10, 1), (11, 10), (12, 11), (20, 1)):
                task = proc / str(pid) / "task" / str(pid)
                task.mkdir(parents=True)
                (proc / str(pid) / "stat").write_text(stat(pid, parent))
                (task / "stat").write_text(stat(pid, parent))
                (task / "wchan").write_text("futex_wait_queue")
                (task / "schedstat").write_text("1000 2000 3")
            vanished = proc / "99"
            vanished.mkdir()
            (vanished / "stat").write_text("truncated")
            result = process_tree(10, proc)
            self.assertEqual([row["pid"] for row in result["processes"]], [10, 11, 12])
            self.assertEqual([row["pid"] for row in result["outside_processes"]], [20])
            self.assertEqual(result["read_races"], 1)
            self.assertEqual(
                result["processes"][1]["io"], {"error": "FileNotFoundError"}
            )
            self.assertEqual(
                result["processes"][1]["tasks"][0]["schedstat"], "1000 2000 3"
            )
            self.assertEqual(
                result["processes"][1]["tasks"][0]["wchan"], "futex_wait_queue"
            )

    def test_unavailable_perf_is_not_zero(self):
        for code, output in (
            (0, "<not supported>;cycles"),
            (0, "<not counted>;instructions"),
            (255, "permission denied"),
        ):
            with self.subTest(output=output):
                self.assertFalse(perf_supported(code, output))
        self.assertTrue(perf_supported(0, "2345;;cycles;100.00"))
        command = ["cargo", "nextest", "run"]
        self.assertEqual(perf_command({"events": []}, Path("unused"), command), command)

    def test_sampler_stops_and_records_its_own_cost(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "samples.jsonl"
            with (
                patch("monitor.COUNTERS", ()),
                patch("monitor.process_tree", return_value={"processes": []}),
            ):
                monitor = Monitor(os.getpid(), path, interval=0.01)
                monitor.start()
                deadline = time.monotonic() + 2
                while monitor.summary["samples"] < 2 and time.monotonic() < deadline:
                    time.sleep(0.01)
                summary = monitor.stop()
            records = [json.loads(line) for line in path.read_text().splitlines()]
            self.assertFalse(monitor.thread.is_alive())
            self.assertEqual(summary["errors"], [])
            self.assertTrue(summary["completed"])
            self.assertGreaterEqual(len(records), 2)
            self.assertGreater(records[-1]["monotonic_ns"], records[0]["monotonic_ns"])
            self.assertGreater(summary["cpu_seconds"], 0)

    def test_sampler_error_is_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            with (
                patch("monitor.COUNTERS", ()),
                patch("monitor.process_tree", side_effect=OSError("probe failed")),
            ):
                monitor = Monitor(os.getpid(), Path(directory) / "samples.jsonl")
                monitor.start()
                monitor.thread.join(timeout=2)
                summary = monitor.stop()
            self.assertEqual(summary["errors"], ["OSError: probe failed"])
            self.assertFalse(summary["completed"])


if __name__ == "__main__":
    unittest.main()
