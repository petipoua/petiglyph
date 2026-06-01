#!/usr/bin/env python3
"""Canonical distribution-target helper and drift checker."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
MATRIX_PATH = REPO_ROOT / "scripts" / "distribution-targets.json"
RELEASE_WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "release.yml"
PYPI_WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "pypi-publish.yml"
NPM_META_PACKAGE_PATH = REPO_ROOT / "npm" / "petiglyph" / "package.json"


def load_matrix() -> dict[str, Any]:
    data = json.loads(MATRIX_PATH.read_text(encoding="utf-8"))
    targets = data.get("targets")
    if not isinstance(targets, list) or not targets:
        raise ValueError(f"{MATRIX_PATH} has no targets list")

    seen_targets: set[str] = set()
    seen_npm_packages: set[str] = set()
    seen_npm_dirs: set[str] = set()
    for entry in targets:
        for key in (
            "target",
            "runner",
            "archive",
            "bin_ext",
            "bin_name",
            "npm_package",
            "npm_dir",
            "pypi",
        ):
            if key not in entry:
                raise ValueError(f"missing key '{key}' in distribution target entry: {entry}")
        target = entry["target"]
        npm_package = entry["npm_package"]
        npm_dir = entry["npm_dir"]
        if target in seen_targets:
            raise ValueError(f"duplicate target in matrix: {target}")
        if npm_package in seen_npm_packages:
            raise ValueError(f"duplicate npm_package in matrix: {npm_package}")
        if npm_dir in seen_npm_dirs:
            raise ValueError(f"duplicate npm_dir in matrix: {npm_dir}")
        seen_targets.add(target)
        seen_npm_packages.add(npm_package)
        seen_npm_dirs.add(npm_dir)

    return data


def iter_targets(matrix: dict[str, Any]) -> list[dict[str, Any]]:
    return list(matrix["targets"])


def parse_release_workflow_matrix() -> list[tuple[str, str, str, str]]:
    text = RELEASE_WORKFLOW_PATH.read_text(encoding="utf-8")
    pattern = re.compile(
        r"- os:\s*(?P<os>[^\n]+)\n"
        r"\s+target:\s*(?P<target>[^\n]+)\n"
        r"\s+archive:\s*(?P<archive>[^\n]+)\n"
        r"\s+bin_ext:\s*\"?(?P<bin_ext>[^\n\"]*)\"?",
        re.MULTILINE,
    )
    return [
        (
            match.group("target").strip(),
            match.group("os").strip(),
            match.group("archive").strip(),
            match.group("bin_ext").strip(),
        )
        for match in pattern.finditer(text)
    ]


def parse_pypi_workflow_matrix() -> list[tuple[str, str, str | None]]:
    text = PYPI_WORKFLOW_PATH.read_text(encoding="utf-8")
    pattern = re.compile(
        r"- os:\s*(?P<os>[^\n]+)\n"
        r"\s+target:\s*(?P<target>[^\n]+)"
        r"(?:\n\s+manylinux:\s*\"?(?P<manylinux>[^\n\"]+)\"?)?",
        re.MULTILINE,
    )
    return [
        (
            match.group("target").strip(),
            match.group("os").strip(),
            match.group("manylinux").strip() if match.group("manylinux") else None,
        )
        for match in pattern.finditer(text)
    ]


def check_release_sync(matrix: dict[str, Any]) -> list[str]:
    canonical = {
        (
            entry["target"],
            entry["runner"],
            entry["archive"],
            entry["bin_ext"],
        )
        for entry in iter_targets(matrix)
    }
    workflow = set(parse_release_workflow_matrix())
    errors: list[str] = []
    if canonical != workflow:
        missing = sorted(canonical - workflow)
        extra = sorted(workflow - canonical)
        if missing:
            errors.append(
                "release.yml missing entries from canonical matrix: "
                + ", ".join(str(item) for item in missing)
            )
        if extra:
            errors.append(
                "release.yml has extra/untracked entries: "
                + ", ".join(str(item) for item in extra)
            )
    return errors


def check_pypi_sync(matrix: dict[str, Any]) -> list[str]:
    canonical_entries = []
    for entry in iter_targets(matrix):
        if not entry["pypi"]:
            continue
        manylinux = entry.get("manylinux")
        canonical_entries.append((entry["target"], entry["runner"], manylinux))

    canonical = set(canonical_entries)
    workflow = set(parse_pypi_workflow_matrix())
    errors: list[str] = []
    if canonical != workflow:
        missing = sorted(canonical - workflow)
        extra = sorted(workflow - canonical)
        if missing:
            errors.append(
                "pypi-publish.yml missing entries from canonical matrix: "
                + ", ".join(str(item) for item in missing)
            )
        if extra:
            errors.append(
                "pypi-publish.yml has extra/untracked entries: "
                + ", ".join(str(item) for item in extra)
            )
    return errors


def check_npm_sync(matrix: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    expected_packages = [entry["npm_package"] for entry in iter_targets(matrix)]
    expected_package_set = set(expected_packages)

    meta = json.loads(NPM_META_PACKAGE_PATH.read_text(encoding="utf-8"))
    optional_deps = meta.get("optionalDependencies")
    if not isinstance(optional_deps, dict):
        errors.append("npm/petiglyph/package.json missing optionalDependencies object")
        return errors

    actual_package_set = set(optional_deps.keys())
    if expected_package_set != actual_package_set:
        missing = sorted(expected_package_set - actual_package_set)
        extra = sorted(actual_package_set - expected_package_set)
        if missing:
            errors.append("npm optionalDependencies missing packages: " + ", ".join(missing))
        if extra:
            errors.append("npm optionalDependencies has extra packages: " + ", ".join(extra))

    for entry in iter_targets(matrix):
        pkg_dir = REPO_ROOT / entry["npm_dir"]
        pkg_json = pkg_dir / "package.json"
        if not pkg_json.exists():
            errors.append(f"missing npm package.json: {pkg_json}")
            continue
        payload = json.loads(pkg_json.read_text(encoding="utf-8"))
        name = payload.get("name")
        if name != entry["npm_package"]:
            errors.append(
                f"npm package name mismatch for {pkg_json}: expected {entry['npm_package']}, got {name}"
            )

    return errors


def print_lines(lines: list[str]) -> None:
    for line in lines:
        print(line)


def format_stage_lines(matrix: dict[str, Any]) -> list[str]:
    return [
        "\t".join((entry["target"], entry["npm_dir"], entry["bin_name"]))
        for entry in iter_targets(matrix)
    ]


def format_npm_bin_paths(matrix: dict[str, Any]) -> list[str]:
    return [f"{entry['npm_dir']}/bin/{entry['bin_name']}" for entry in iter_targets(matrix)]


def format_npm_package_dirs(matrix: dict[str, Any]) -> list[str]:
    return [entry["npm_dir"] for entry in iter_targets(matrix)]


def main() -> int:
    parser = argparse.ArgumentParser(description="Distribution target matrix helper")
    parser.add_argument("--stage-lines", action="store_true", help="emit target<TAB>npm_dir<TAB>bin_name")
    parser.add_argument("--npm-bin-paths", action="store_true", help="emit npm binary paths")
    parser.add_argument("--npm-package-dirs", action="store_true", help="emit npm package directories")
    parser.add_argument("--check-sync", action="store_true", help="validate workflows/npm metadata against canonical matrix")
    args = parser.parse_args()

    if not any((args.stage_lines, args.npm_bin_paths, args.npm_package_dirs, args.check_sync)):
        parser.error("select at least one output/check mode")

    try:
        matrix = load_matrix()
    except Exception as exc:  # noqa: BLE001
        print(f"error: failed to load matrix: {exc}", file=sys.stderr)
        return 1

    if args.stage_lines:
        print_lines(format_stage_lines(matrix))

    if args.npm_bin_paths:
        print_lines(format_npm_bin_paths(matrix))

    if args.npm_package_dirs:
        print_lines(format_npm_package_dirs(matrix))

    if args.check_sync:
        errors: list[str] = []
        errors.extend(check_release_sync(matrix))
        errors.extend(check_pypi_sync(matrix))
        errors.extend(check_npm_sync(matrix))
        if errors:
            for err in errors:
                print(f"matrix-sync error: {err}", file=sys.stderr)
            return 1
        print("matrix sync check passed")

    return 0


if __name__ == "__main__":
    sys.exit(main())
