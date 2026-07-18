#!/usr/bin/env python3
"""Independent corpus enumeration (double-entry accounting check).

Counts commands in build/wast-json/*.json directly, independent of the
spec-runner, and (optionally) cross-checks a runner ledger. Exits nonzero
on any mismatch, so CI can gate on both sides agreeing.
"""

import argparse
import collections
import glob
import json
import os
import sys


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default="build/wast-json")
    ap.add_argument("--ledger", help="runner ledger JSON to cross-check")
    args = ap.parse_args()

    files = sorted(glob.glob(os.path.join(args.dir, "*.json")))
    if not files:
        print(f"no JSON files under {args.dir}", file=sys.stderr)
        return 2

    total = 0
    text_modules = 0
    by_type = collections.Counter()
    per_file = {}
    for p in files:
        with open(p, encoding="utf-8") as f:
            doc = json.load(f)
        n = 0
        for c in doc["commands"]:
            total += 1
            n += 1
            by_type[c["type"]] += 1
            if c.get("module_type") == "text":
                text_modules += 1
        per_file[os.path.basename(p)] = n

    print(f"ENUM files={len(files)} total={total} module_type_text={text_modules}")
    for k, v in sorted(by_type.items()):
        print(f"  {k}: {v}")

    ok = True
    if args.ledger:
        with open(args.ledger, encoding="utf-8") as f:
            ledger = json.load(f)
        lt = ledger["totals"]
        checks = [
            ("commands", lt["commands"], total),
            ("unsupported", lt["unsupported"], text_modules),
            ("files", lt["files"], len(files)),
            ("pass+fail+unsupported", lt["pass"] + lt["fail"] + lt["unsupported"], total),
        ]
        for name, got, want in checks:
            status = "ok" if got == want else "MISMATCH"
            if got != want:
                ok = False
            print(f"CROSS-CHECK {name}: ledger={got} enum={want} {status}")
        # The ledger's file set must be exactly the corpus file set
        # (no omissions, no duplicates), not merely count-compatible.
        ledger_names = [lf["file"] for lf in ledger["files"]]
        if sorted(ledger_names) != sorted(per_file):
            missing = sorted(set(per_file) - set(ledger_names))
            extra = sorted(set(ledger_names) - set(per_file))
            dupes = sorted({n for n in ledger_names if ledger_names.count(n) > 1})
            print(f"CROSS-CHECK file-set MISMATCH: missing={missing} extra={extra} dupes={dupes}")
            ok = False
        for lf in ledger["files"]:
            want = per_file.get(lf["file"])
            if want is None or lf["total"] != want:
                print(f"CROSS-CHECK per-file MISMATCH: {lf['file']} ledger={lf['total']} enum={want}")
                ok = False
        # Every UNSUPPORTED row must be attributed to the text-format class.
        bad_rows = [
            (lf["file"], r["line"], r["reason"])
            for lf in ledger["files"]
            for r in lf["non_pass_rows"]
            if r["verdict"] == "UNSUPPORTED" and "text-format" not in r["reason"]
        ]
        n_unsup_rows = sum(
            1
            for lf in ledger["files"]
            for r in lf["non_pass_rows"]
            if r["verdict"] == "UNSUPPORTED"
        )
        print(f"CROSS-CHECK unsupported rows itemized: {n_unsup_rows} (off-class: {len(bad_rows)})")
        if n_unsup_rows != lt["unsupported"] or bad_rows:
            ok = False

    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
