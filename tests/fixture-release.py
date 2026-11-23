#!/usr/bin/env python3
"""Release adapter with a disposable filesystem destination."""
import hashlib
import json
import os
import sys
import time
from pathlib import Path

request = json.load(sys.stdin)
phase = sys.argv[1]
artifact = Path(request["artifact"])
if phase == "build":
    source = Path(request["source"])
    artifact.write_bytes((source / "result.txt").read_bytes())
elif phase == "verify":
    assert "sha256:" + hashlib.sha256(artifact.read_bytes()).hexdigest() == request["digest"]
else:
    live = Path(os.environ["SWARM_TEST_LIVE"])
    if phase == "status":
        if Path(str(live) + ".unavailable").exists():
            raise SystemExit(1)
        print(live.read_text())
    elif phase == "deploy":
        if Path(str(live) + ".forbid-promotion").exists():
            Path(str(live) + ".unexpected-promotion").touch()
            raise SystemExit(1)
        live.write_text(json.dumps({
            "digest": request["digest"],
            "healthy": not Path(str(live) + ".unhealthy").exists(),
        }))
        if Path(str(live) + ".interrupt").exists():
            # Make the external effect visible before the process returns.
            time.sleep(4)
            raise SystemExit(1)
    elif phase == "rollback":
        live.write_text(json.dumps({"digest": request["previous"], "healthy": True}))
    else:
        raise ValueError(phase)
