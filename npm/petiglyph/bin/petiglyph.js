#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

function detectLinuxLibc() {
  if (process.platform !== "linux") {
    return null;
  }

  try {
    const report = process.report?.getReport?.();
    if (report?.header?.glibcVersionRuntime) {
      return "gnu";
    }
  } catch (_) {
    // Ignore and continue with command-based fallback detection.
  }

  const probes = [
    ["getconf", ["GNU_LIBC_VERSION"]],
    ["ldd", ["--version"]],
  ];

  for (const [cmd, args] of probes) {
    const result = spawnSync(cmd, args, { encoding: "utf8" });
    const text = `${result.stdout || ""}\n${result.stderr || ""}`.toLowerCase();
    if (text.includes("musl")) {
      return "musl";
    }
    if (text.includes("glibc") || text.includes("gnu libc")) {
      return "gnu";
    }
  }

  return null;
}

function resolvePlatformPackage() {
  const platform = process.platform;
  const arch = process.arch;
  const libc = platform === "linux" ? detectLinuxLibc() : null;

  const key = [platform, arch, libc || "none"].join("|");
  const mapping = {
    "linux|x64|gnu": {
      packageName: "@petiglyph/petiglyph-linux-x64-gnu",
      binaryRelativePath: "bin/petiglyph",
    },
    "linux|arm64|gnu": {
      packageName: "@petiglyph/petiglyph-linux-arm64-gnu",
      binaryRelativePath: "bin/petiglyph",
    },
    "linux|x64|musl": {
      packageName: "@petiglyph/petiglyph-linux-x64-musl",
      binaryRelativePath: "bin/petiglyph",
    },
    "linux|arm64|musl": {
      packageName: "@petiglyph/petiglyph-linux-arm64-musl",
      binaryRelativePath: "bin/petiglyph",
    },
    "darwin|x64|none": {
      packageName: "@petiglyph/petiglyph-darwin-x64",
      binaryRelativePath: "bin/petiglyph",
    },
    "darwin|arm64|none": {
      packageName: "@petiglyph/petiglyph-darwin-arm64",
      binaryRelativePath: "bin/petiglyph",
    },
    "win32|x64|none": {
      packageName: "@petiglyph/petiglyph-win32-x64-msvc",
      binaryRelativePath: "bin/petiglyph.exe",
    },
    "win32|arm64|none": {
      packageName: "@petiglyph/petiglyph-win32-arm64-msvc",
      binaryRelativePath: "bin/petiglyph.exe",
    },
  };

  const selected = mapping[key];
  if (selected) {
    return selected;
  }

  if (platform === "linux") {
    throw new Error(
      `unsupported Linux libc/platform combination: arch=${arch}, libc=${libc || "unknown"}`,
    );
  }
  throw new Error(`unsupported platform combination: platform=${platform}, arch=${arch}`);
}

function resolveBinaryPath(packageName, binaryRelativePath) {
  const packageJsonPath = require.resolve(`${packageName}/package.json`);
  return path.join(path.dirname(packageJsonPath), binaryRelativePath);
}

function run() {
  const { packageName, binaryRelativePath } = resolvePlatformPackage();

  let binaryPath;
  try {
    binaryPath = resolveBinaryPath(packageName, binaryRelativePath);
  } catch (error) {
    const lines = [
      "petiglyph: no compatible native package was found for this environment.",
      `Attempted package: ${packageName}`,
      `Error: ${error instanceof Error ? error.message : String(error)}`,
      "",
      "Try reinstalling with optional dependencies enabled, or install petiglyph from GitHub releases.",
    ];
    console.error(lines.join("\n"));
    process.exit(1);
  }

  if (!fs.existsSync(binaryPath)) {
    console.error(`petiglyph: expected native binary is missing: ${binaryPath}`);
    process.exit(1);
  }

  const child = spawnSync(binaryPath, process.argv.slice(2), {
    stdio: "inherit",
    windowsHide: false,
  });

  if (child.error) {
    console.error(`petiglyph: failed to execute native binary: ${child.error.message}`);
    process.exit(1);
  }

  if (typeof child.status === "number") {
    process.exit(child.status);
  }

  process.exit(1);
}

run();
