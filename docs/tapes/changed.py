#!/usr/bin/env python3
"""Which screenshots and demos a change calls for, by docs/tapes/views.toml.

    changed.py --base <sha> [--head <sha>]   files changed on head since it left base
    changed.py --files a.rs b.rs             these files
    changed.py --all                         everything
    changed.py --only history sets           these screenshots and demos, by name
    changed.py --check                       every path views.toml names exists

Prints `screenshots=<names>` and `tapes=<names>`, space-separated, in the form of
$GITHUB_OUTPUT. A base that git does not have (a new branch's push names 000…0) means
everything. Run with python3 -I.
"""

import argparse
import fnmatch
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VIEWS = Path(__file__).resolve().with_name("views.toml")


def load():
    with VIEWS.open("rb") as f:
        views = tomllib.load(f)
    return views["all"]["paths"], views.get("screenshot", {}), views.get("tape", {})


def changed_files(base, head):
    """The files changed from base to head, or None when git cannot tell."""
    if not base or set(base) == {"0"}:
        return None
    probe = subprocess.run(
        ["git", "-C", str(ROOT), "cat-file", "-e", f"{base}^{{commit}}"], capture_output=True, check=False
    )
    if probe.returncode != 0:
        return None
    diff = subprocess.run(
        ["git", "-C", str(ROOT), "diff", "--name-only", f"{base}...{head}"],
        capture_output=True,
        text=True,
        check=True,
    )
    return [line for line in diff.stdout.splitlines() if line]


def matches(files, patterns):
    return any(fnmatch.fnmatchcase(f, p) for f in files for p in patterns)


def check(everything, screenshots, tapes):
    """Names each pattern that matches no file in the repository: a view's file was moved."""
    listed = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "--cached", "--others", "--exclude-standard"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    groups = [("all", everything)]
    groups += [(f"screenshot.{n}", v["paths"]) for n, v in screenshots.items()]
    groups += [(f"tape.{n}", v["paths"]) for n, v in tapes.items()]
    stale = [(g, p) for g, patterns in groups for p in patterns if not any(fnmatch.fnmatchcase(f, p) for f in listed)]
    for name in tapes:
        if not (VIEWS.parent / f"{name}.tape").is_file():
            stale.append((f"tape.{name}", f"docs/tapes/{name}.tape"))
    for group, pattern in stale:
        print(f"views.toml: [{group}] {pattern} matches no file", file=sys.stderr)
    return 1 if stale else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument("--files", nargs="*")
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    everything, screenshots, tapes = load()
    if args.check:
        return check(everything, screenshots, tapes)
    if args.only is not None:
        unknown = [n for n in args.only if n not in screenshots and n not in tapes]
        if unknown:
            print(f"changed.py: no screenshot or tape {' '.join(unknown)}", file=sys.stderr)
            return 2
        print(f"screenshots={' '.join(n for n in args.only if n in screenshots)}")
        print(f"tapes={' '.join(n for n in args.only if n in tapes)}")
        return 0
    files = None if args.all else args.files if args.files is not None else changed_files(args.base, args.head)
    if files is None or matches(files, everything):
        shots, demos = list(screenshots), list(tapes)
    else:
        shots = [n for n, v in screenshots.items() if matches(files, v["paths"])]
        demos = [n for n, v in tapes.items() if matches(files, v["paths"])]
    print(f"screenshots={' '.join(shots)}")
    print(f"tapes={' '.join(demos)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
