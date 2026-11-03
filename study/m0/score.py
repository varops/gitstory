"""Score the M0 control-arm ratings.

Usage: python3 study/m0/score.py STUDY_DIR

Reads STUDY_DIR/key.jsonl (written by `git story land --study-baseline`) and
STUDY_DIR/ratings.csv (filled in by raters), un-blinds the A/B labels and
prints each rate with its 95% Wilson interval. Overlapping intervals mean
these ratings do not tell the arms apart.
"""
import csv
import json
import math
import sys
from pathlib import Path

Z = 1.959963984540054


def wilson(k, n):
    if n == 0:
        return (0.0, 1.0)
    p = k / n
    d = 1 + Z * Z / n
    c = (p + Z * Z / (2 * n)) / d
    h = Z * math.sqrt(p * (1 - p) / n + Z * Z / (4 * n * n)) / d
    return (max(0.0, c - h), min(1.0, c + h))


def rate(k, n):
    lo, hi = wilson(k, n)
    return f"{k}/{n} [{lo:.2f}, {hi:.2f}]"


def main(d):
    d = Path(d)
    key = {}
    for line in (d / "key.jsonl").read_text(encoding="utf-8").splitlines():
        if line.strip():
            k = json.loads(line)
            key[k["commit"]] = k
    rows = list(csv.DictReader((d / "ratings.csv").open(encoding="utf-8")))
    arms = {"gitstory": {"accuracy": [], "usefulness": []}, "baseline": {"accuracy": [], "usefulness": []}}
    prefer = {"gitstory": 0, "baseline": 0, "same": 0}
    unknown = []
    for r in rows:
        k = key.get(r["commit"].strip()[:12])
        if not k:
            unknown.append(r["commit"])
            continue
        for side in ("a", "b"):
            arm = k[side]
            arms[arm]["accuracy"].append(int(r[f"{side}_accuracy"]))
            arms[arm]["usefulness"].append(int(r[f"{side}_usefulness"]))
        p = r["prefer"].strip().upper()
        prefer[k["a"] if p == "A" else k["b"] if p == "B" else "same"] += 1

    print(f"{len(rows) - len(unknown)} ratings over {len({r['commit'] for r in rows})} commits")
    if unknown:
        print(f"skipped, not in key.jsonl: {', '.join(unknown)}")
    print()
    for arm, s in arms.items():
        for m, xs in s.items():
            # Share of ratings at 4 or 5, so the interval is a proportion.
            good = sum(1 for x in xs if x >= 4)
            mean = sum(xs) / len(xs) if xs else float("nan")
            print(f"{arm:<9} {m:<10} mean {mean:.2f}   rated 4-5: {rate(good, len(xs))}")
    decided = prefer["gitstory"] + prefer["baseline"]
    print()
    print(f"prefer gitstory   {rate(prefer['gitstory'], decided)}   (ties left out: {prefer['same']})")
    lo, hi = wilson(prefer["gitstory"], decided)
    if lo > 0.5:
        print("these ratings favor gitstory")
    elif hi < 0.5:
        print("these ratings favor the baseline")
    else:
        print("these ratings do not tell the arms apart")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
