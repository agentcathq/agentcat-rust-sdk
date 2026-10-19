#!/usr/bin/env python3
"""Builds tests/parity/manifest.tsv and tests/parity/NOT_PORTED.md.

Inputs:
  1. `vitest list --json` output for the v1, v2 and e2e-http projects of
     agentcat-typescript-sdk at 9b56e9e (the authoritative test names).
  2. docs/research/06a-06c-ts-test-index.md (the port / adapt / N/A call per row).

Usage: build-manifest.py <out_dir> <index_dir> <vitest_json>...
"""
import hashlib
import json
import re
import sys
from collections import OrderedDict

KEYWORDS = {
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod",
    "move", "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait",
    "true", "type", "unsafe", "use", "where", "while", "abstract", "become", "box", "do",
    "final", "macro", "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
}

# Files that run outside the three vitest projects; their rows come from the index alone.
INDEX_ONLY = {"workers/event-delivery.test.ts"}


def ident(segment: str) -> str:
    s = re.sub(r"[^a-z0-9]+", "_", segment.lower()).strip("_") or "t"
    if s[0].isdigit():
        s = "t_" + s
    if len(s) > 80:
        s = s[:70].rstrip("_") + "_" + hashlib.sha256(segment.encode()).hexdigest()[:8]
    if s in KEYWORDS:
        s += "_"
    return s


def file_module(ts_file: str) -> str:
    return ident(ts_file.removesuffix(".test.ts").replace("/", "_"))


def parse_index(index_dir):
    """file -> list of (count, title, disposition, verdict text)."""
    rows = OrderedDict()
    current = None
    for part in ("06a", "06b", "06c"):
        with open(f"{index_dir}/{part}-ts-test-index.md", encoding="utf-8") as fh:
            for line in fh:
                m = re.match(r"^### src/tests/(\S+\.test\.ts)", line)
                if m:
                    current = m.group(1)
                    rows.setdefault(current, [])
                    continue
                if line.startswith("### "):
                    current = None
                    continue
                if current is None:
                    continue
                m = re.match(r"^\| (\d+)(?: ×(\d+))? \| (.*) \|$", line.rstrip("\n"))
                if not m:
                    continue
                cells = [c.strip().replace("\\|", "|") for c in re.split(r"(?<!\\) \| ", m.group(3))]
                verdict = cells[-1]
                low = verdict.lower()
                if low.startswith("n/a"):
                    disp = "na"
                elif low.startswith("adapt"):
                    disp = "adapt"
                elif low.startswith("port"):
                    disp = "port"
                else:
                    raise SystemExit(f"{current} row {m.group(1)}: unknown verdict {verdict!r}")
                rows[current].append((int(m.group(2) or 1), cells[0], disp, verdict))
    return rows


def expand_ellipses(titles):
    """Index titles abbreviate a repeated describe path as "... > it"."""
    out = []
    for t in titles:
        for mark in ("... > ", "… > "):
            if t.startswith(mark) and out:
                t = out[-1].rsplit(" > ", 1)[0] + " > " + t[len(mark):]
        out.append(t)
    return out


def last_segment(title: str) -> str:
    return title.split(" > ")[-1].strip().strip("'\"`")


def main():
    out_dir, index_dir, *vitest_files = sys.argv[1:]
    index = parse_index(index_dir)
    cases = OrderedDict()
    for path in vitest_files:
        with open(path, encoding="utf-8") as fh:
            for case in json.load(fh):
                f = case["file"].split("/src/tests/", 1)[1]
                cases.setdefault(f, []).append(case["name"])

    manifest = []
    problems = []
    for f, rows in index.items():
        expanded = [r for r in rows for _ in range(r[0])]
        names = cases.get(f)
        if names is None:
            if f not in INDEX_ONLY and rows:
                problems.append(f"{f}: no vitest cases")
                continue
            names = expand_ellipses([r[1] for r in expanded])
        if len(expanded) != len(names):
            if len(expanded) - len(names) == 1 and f == "error-capture.test.ts":
                # it.skip: vitest does not list it; take its describe path from its neighbour.
                idx = next(i for i, r in enumerate(expanded) if "in_app frames correctly" in r[1])
                prefix = names[idx - 1].rsplit(" > ", 1)[0]
                skipped_it = last_segment(expanded[idx][1]).removesuffix(" (skipped)")
                names = names[:idx] + [f"{prefix} > {skipped_it}"] + names[idx:]
            else:
                problems.append(f"{f}: index has {len(expanded)} cases, vitest has {len(names)}")
                continue
        # describe.each matrices list lanes outer, scenarios inner, while the index
        # lists scenarios with a ×N count: pair those by title. Every other file
        # lists cases in vitest order.
        paired = []
        if not f.startswith("e2e-http/"):
            paired = list(zip(names, expanded))
        else:
            by_last = {}
            for r in expanded:
                by_last.setdefault(last_segment(r[1]), []).append(r)
            for n in names:
                pool = by_last.get(last_segment(n))
                if not pool:
                    problems.append(f"{f}: no index row for {n!r}")
                    break
                paired.append((n, pool[0]))
        used = {}
        for n, r in paired:
            path = "::".join([file_module(f)] + [ident(s) for s in n.split(" > ")])
            k = used.get(path, 0) + 1
            used[path] = k
            if k > 1:
                path = f"{path}_{k}"
            manifest.append((f, n, r[2], path, r[3]))

    if problems:
        print("\n".join(problems), file=sys.stderr)
        raise SystemExit(1)

    with open(f"{out_dir}/manifest.tsv", "w", encoding="utf-8") as fh:
        fh.write("ts_file\tts_name\tdisposition\trust_test\n")
        for f, n, d, p, _ in manifest:
            fh.write(f"{f}\t{n}\t{d}\t{p}\n")
    with open(f"{out_dir}/NOT_PORTED.md", "w", encoding="utf-8") as fh:
        fh.write("# TS tests with no Rust port\n\n")
        fh.write("Generated by `scripts/parity/build-manifest.py`. Each row is an `N/A` row of\n")
        fh.write("`docs/research/06a`–`06c`. Edit the index, then regenerate.\n\n")
        fh.write("| TS file | TS test | Reason |\n|---|---|---|\n")
        previous = ""
        for f, n, d, _, note in manifest:
            if d == "na":
                reason = note.split(":", 1)[1].strip() if ":" in note else note
                if reason.rstrip(".").lower() == "same":
                    reason = previous
                previous = reason
                name = n.replace("|", "\\|")
                fh.write(f"| `{f}` | {name} | {reason} |\n")
    counts = {}
    for _, _, d, _, _ in manifest:
        counts[d] = counts.get(d, 0) + 1
    print(f"{len(manifest)} cases: {counts}")


if __name__ == "__main__":
    main()
