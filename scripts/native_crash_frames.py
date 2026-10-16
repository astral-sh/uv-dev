"""Export frame identities without runtime arguments, locals, or environment values."""

import json
import os

import gdb

threads = list(gdb.selected_inferior().threads())
selected = gdb.selected_thread()
threads.sort(key=lambda thread: (thread != selected, thread.ptid))
result = {"schema_version": 1, "threads": [], "threads_truncated": len(threads) > 16}
for thread in threads[:16]:
    record = {"lwp": thread.ptid[1], "selected": thread == selected, "frames": []}
    try:
        thread.switch()
        frame = gdb.newest_frame()
        while frame is not None and len(record["frames"]) < 32:
            name = frame.name()
            if name is not None:
                name = "".join(
                    character for character in name[:256] if 32 <= ord(character) < 127
                )
            record["frames"].append(
                {"pc": hex(frame.pc()), "function": name, "type": frame.type()}
            )
            frame = frame.older()
        record["frames_truncated"] = frame is not None
    except gdb.error:
        record["error"] = "frame_unavailable"
    result["threads"].append(record)

with open(os.environ["UV_CRASH_FRAMES_PATH"], "x", encoding="utf-8") as output:
    json.dump(result, output, separators=(",", ":"))
