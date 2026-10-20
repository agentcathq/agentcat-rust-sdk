#!/usr/bin/env python3
"""Mutation score per group from cargo-mutants output directories.

Usage: mutants-score.py --min-core N --min-adapter N DIR...
Each DIR holds caught.txt, missed.txt, timeout.txt (cargo-mutants' mutants.out).
Score = (caught + timeout) / (caught + timeout + missed); unviable mutants are left out.
Groups: adapter = src/tracked.rs and src/tool_call.rs; core = every other src/ file.
"""
import argparse
import pathlib
import sys

ADAPTER = {"src/tracked.rs", "src/tool_call.rs"}


def lines(directory, name):
    path = pathlib.Path(directory, name)
    return path.read_text(encoding="utf-8").splitlines() if path.exists() else []


def group(mutant):
    return "adapter" if mutant.split(":", 1)[0] in ADAPTER else "core"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--min-core", type=float, required=True)
    parser.add_argument("--min-adapter", type=float, required=True)
    parser.add_argument("dirs", nargs="+")
    args = parser.parse_args()
    stats = {"core": [0, 0], "adapter": [0, 0]}
    missed = []
    for d in args.dirs:
        for m in lines(d, "caught.txt") + lines(d, "timeout.txt"):
            stats[group(m)][0] += 1
        for m in lines(d, "missed.txt"):
            stats[group(m)][1] += 1
            missed.append(m)
    ok = True
    for name, minimum in (("core", args.min_core), ("adapter", args.min_adapter)):
        caught, miss = stats[name]
        total = caught + miss
        score = 100.0 if total == 0 else 100.0 * caught / total
        print(f"{name}: {score:.1f}% ({caught} caught, {miss} missed), minimum {minimum}%")
        ok = ok and score >= minimum
    if missed:
        print("\nMissed mutants:\n" + "\n".join(sorted(missed)))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
