#!/usr/bin/env python3
"""A stand-in for pixidl-native-host, used only by the end-to-end test.

Speaks the native-messaging framing (32-bit native-endian length + UTF-8
JSON) on stdin/stdout. Every request is appended to requests.jsonl in the
state directory (argv[1] or $PIXIDL_FAKE_HOST_DIR). The file `mode` there
picks the answers:

  ok           ping and add_download succeed
  unavailable  every request fails with app_unavailable
  add_fail     ping succeeds, add_download fails with internal
  crash        exits without answering (like a missing or broken host)
"""

import json
import os
import struct
import sys
import time


def state_dir():
    if len(sys.argv) > 1 and os.path.isdir(sys.argv[1]):
        return sys.argv[1]
    return os.environ.get("PIXIDL_FAKE_HOST_DIR") or os.path.dirname(os.path.abspath(__file__))


def read_message():
    raw = sys.stdin.buffer.read(4)
    if len(raw) < 4:
        return None
    (length,) = struct.unpack("=I", raw)
    return json.loads(sys.stdin.buffer.read(length).decode("utf-8"))


def write_message(obj):
    data = json.dumps(obj).encode("utf-8")
    sys.stdout.buffer.write(struct.pack("=I", len(data)))
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()


def main():
    directory = state_dir()
    try:
        with open(os.path.join(directory, "mode"), encoding="utf-8") as f:
            mode = f.read().strip() or "ok"
    except OSError:
        mode = "ok"
    msg = read_message()
    if msg is None:
        return
    with open(os.path.join(directory, "requests.jsonl"), "a", encoding="utf-8") as log:
        log.write(json.dumps({"t": time.time(), "mode": mode, "request": msg}) + "\n")
    if mode == "crash":
        return
    base = {"version": 1, "id": msg.get("id")}
    kind = msg.get("type")
    if mode == "unavailable":
        write_message({**base, "success": False, "error": {"code": "app_unavailable", "message": "pixidl is not running"}})
        return
    if kind == "ping":
        write_message({**base, "success": True, "app": "pixidl", "app_version": "9.9.9-fake", "protocol_version": 1,
                       "integration_enabled": True, "accent_color": "#7C3AED", "language": "en"})
    elif kind == "add_download":
        if mode == "add_fail":
            write_message({**base, "success": False, "error": {"code": "internal", "message": "fake failure"}})
        else:
            url = msg.get("payload", {}).get("url", "")
            name = msg.get("payload", {}).get("filename") or url.rstrip("/").split("/")[-1]
            write_message({**base, "success": True, "download_id": "fake-1", "filename": name, "engine": "http"})
    elif kind == "get_status":
        write_message({**base, "success": True, "active": 0, "queued": 0, "download_bytes_per_second": 0, "downloads": []})
    else:
        write_message({**base, "success": False, "error": {"code": "unknown_type", "message": kind or ""}})


if __name__ == "__main__":
    main()
