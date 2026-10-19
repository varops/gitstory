"""Deterministic stand-in for a model, for `[model] provider = "command"`.

Reads {"model", "system", "user", "schema"} on stdin and writes JSON that
follows the schema. Every request is appended to $MOCK_LOG when set.
"""
import json
import os
import sys

req = json.load(sys.stdin)
log = os.environ.get("MOCK_LOG")
if log:
    with open(log, "a", encoding="utf-8") as f:
        f.write(json.dumps(req) + "\n")
if os.environ.get("MOCK_FAIL"):
    sys.stderr.write("mock model failure\n")
    sys.exit(3)

user = json.loads(req["user"])

if "verdicts" in req["schema"]["properties"]:
    out = []
    for c in user["claims"]:
        text = c["claim"]
        sup = "no" if "UNSUPPORTED" in text else "partly" if "MAYBE" in text else "yes"
        out.append({"index": c["index"], "supported": sup, "note": ""})
    print(json.dumps({"verdicts": out}))
    sys.exit(0)

events = user.get("events", [])
landed = {f["path"] for f in user.get("diff", [])}


def text(e):
    return e.get("excerpt") or e["fact"]


def claim(c, t, status, ids):
    return {"claim": c, "type": t, "status": status, "evidence": ids}


goal = next((e for e in events if e["type"] == "goal"), events[0] if events else None)
decisions, rejected, discarded, summary_ev = [], [], [], []
for e in events:
    if e["type"] in ("decision", "reason", "constraint"):
        decisions.append(claim(text(e), "stated", "implemented", [e["id"]]))
    elif e["type"] == "abandon":
        rejected.append(claim(text(e), "observed", "rejected", [e["id"]]))
    elif e["type"] == "test":
        summary_ev.append(claim(e["fact"], "observed", "none", [e["id"]]))
    elif e["type"] == "attempt" and e.get("paths"):
        paths = e["paths"]
        if user.get("drafts"):
            # Reconciliation: only what the landed diff shows is a decision.
            kept = [p for p in paths if p in landed]
            gone = [p for p in paths if p not in landed]
            if kept:
                decisions.append(claim("Changed " + ", ".join(kept), "observed", "implemented", [e["id"]]))
            if gone:
                discarded.append(claim("Tried " + ", ".join(gone), "observed", "discarded", [e["id"]]))
        else:
            decisions.append(claim("Changed " + ", ".join(paths), "observed", "implemented", [e["id"]]))
first = [events[0]["id"]] if events else []
story = {
    "summary": user["commit"]["subject"] or "Work in progress",
    "goal": claim(text(goal) if goal else "unknown", "stated", "none", [goal["id"]] if goal else []),
    "decisions": decisions + [claim("UNSUPPORTED invented decision", "stated", "implemented", first),
                              claim("Cites nothing real", "stated", "implemented", ["ev_doesnotexist"])],
    "rejected": rejected,
    "discarded": discarded,
    "evidence_summary": summary_ev,
    "hypotheses": [claim("MAYBE the author preferred simplicity", "inferred", "none", first)],
    "limitations": [],
    "unknowns": [],
    "paths": sorted(landed),
}
print(json.dumps(story))
