#!/usr/bin/env python3
"""Fake stub for ProcessTransport tests.

Speaks the mu-protocol wire on stdio: one JSON request per line in,
one JSON reply per line out. `op` picks the scripted behavior so each
test can pin one failure shape (inventory §3.3).
"""

import json
import sys
import time


def reply(req, result=None, error=None):
    out = {"_request_id": req.get("_request_id")}
    if error is not None:
        out["error"] = error
    else:
        out["result"] = result
    print(json.dumps(out), flush=True)


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        req = json.loads(line)
        op = req.get("op", "")
        if op == "echo":
            reply(req, {"op": op, "params": {k: v for k, v in req.items() if k not in ("_request_id", "op")}})
        elif op == "refuse":
            reply(req, error={"code": "UNSUPPORTED_CAPABILITY", "message": "stub says no"})
        elif op == "unknown":
            reply(req, error={"code": "ACTION_OUTCOME_UNKNOWN", "message": "dispatch failed; do not replay"})
        elif op == "offline":
            reply(req, error={"code": "DEVICE_OFFLINE", "message": "stub is gone"})
        elif op in ("action", "observe"):
            # mutating (`action`) and reading (`observe`) sleeps, for the
            # deadline-classification split
            time.sleep(req.get("sleep_s", 60))
            reply(req, {"slept": True})
        elif op == "die":
            sys.exit(1)
        else:
            reply(req, error={"code": "INVALID_ARGUMENT", "message": f"unknown op {op}"})


if __name__ == "__main__":
    main()
