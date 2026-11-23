"""Independent evaluator for the daemon and browser tests."""
import json
import sys
from pathlib import Path

case = json.load(sys.stdin)
value = Path("result.txt")
implementation = value.read_text() if value.exists() else "starting"
relevance, latency = {"starting": (3, 2), "wrong": (10, 8), "correct": (7, 3)}[implementation]
if Path("mixed.txt").exists():
    relevance += 1
print(json.dumps({
    "pass": True,
    "observed": implementation,
    "metrics": {"relevance": relevance, "latency": latency + (case["sample"] - 1) / 10},
}))
