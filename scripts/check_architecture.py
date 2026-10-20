#!/usr/bin/env python3
"""Check engine/desktop boundaries; run from any working directory."""

from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
# sysinfo legitimately uses Foundation/IOKit for macOS metrics. Reject desktop
# frameworks, rather than all Objective-C dependencies.
DESKTOP_PACKAGE = re.compile(r"^(?:pinch-macos|pinch-ui|gpui(?:[-_].*)?|objc2-app-kit|objc2-user-notifications)$")
NATIVE_SOURCE = re.compile(r"\b(?:pinch_macos|gpui(?:_\w+)?|objc2(?:_\w+)?|block2)\b")
SOCKET_BACKEND = re.compile(r"\b(?:lsof|netstat)\b|/proc/net/")


def check_sources(root):
    errors = []
    source = root / "crates/engine/src"
    paths = [path for crate in ("engine", "client", "proto", "cli")
             for path in (root / "crates" / crate / "src").rglob("*.rs")]
    for path in sorted(paths):
        relative = path.relative_to(source) if path.is_relative_to(source) else None
        if relative and relative.parts[0] == "desktop":
            continue
        text = path.read_text()
        for pattern, explanation in (
            (NATIVE_SOURCE, "desktop/native imports belong in src/desktop/"),
            (SOCKET_BACKEND, "socket backend details belong in src/discovery/"),
        ):
            if pattern is SOCKET_BACKEND and relative and relative.parts[0] == "discovery":
                continue
            for match in pattern.finditer(text):
                line = text.count("\n", 0, match.start()) + 1
                errors.append(f"{path.relative_to(root)}:{line}: {explanation} ({match.group()})")
    return errors


def check_dependencies(tree):
    return [f"headless dependency graph includes desktop package: {name}"
            for name in sorted(set(line.split()[0] for line in tree.splitlines() if line.strip()))
            if DESKTOP_PACKAGE.fullmatch(name)]


def main():
    errors = check_sources(ROOT)
    tree = subprocess.check_output(
        ["cargo", "tree", "--locked", "-p", "pinch-engine", "-p", "pinch-cli",
         "--no-default-features", "--edges", "normal,build", "--prefix", "none",
         "--format", "{p}"], cwd=ROOT, text=True,
    )
    errors.extend(check_dependencies(tree))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("Engine boundaries checked (sources and headless dependency graph).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
