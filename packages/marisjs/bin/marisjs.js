#!/usr/bin/env node

// Locates and executes the platform-native marisjs binary.
// The binary is installed as an optionalDependency of this package,
// with `os` and `cpu` fields so npm only installs the matching one.

import { spawnSync } from "child_process";
import { createRequire } from "module";
import { existsSync } from "fs";
import path from "path";
import { fileURLToPath } from "url";

const require = createRequire(import.meta.url);
const EXE = process.platform === "win32" ? "marisjs.exe" : "marisjs";

function getPlatformPackage() {
  const p = process.platform;
  const a = process.arch;
  if (p === "linux" && a === "x64") return "marisjs-linux-x64";
  if (p === "linux" && a === "arm64") return "marisjs-linux-arm64";
  if (p === "darwin" && a === "x64") return "marisjs-darwin-x64";
  if (p === "darwin" && a === "arm64") return "marisjs-darwin-arm64";
  if (p === "win32" && a === "x64") return "marisjs-win32-x64";
  throw new Error(
    `marisjs: unsupported platform ${p} ${a}. ` +
    `Supported: linux x64/arm64, macos x64/arm64, windows x64.`
  );
}

function findBinary() {
  const pkg = getPlatformPackage();

  const candidates = [
    () => {
      const pkgDir = path.dirname(require.resolve(`${pkg}/package.json`));
      return path.join(pkgDir, "bin", EXE);
    },
    () => path.join(process.cwd(), "node_modules", pkg, "bin", EXE),
    () => path.join(
      path.dirname(fileURLToPath(import.meta.url)),
      "..", "..", pkg, "bin", EXE
    ),
  ];

  for (const candidate of candidates) {
    try {
      const binPath = candidate();
      if (existsSync(binPath)) return binPath;
    } catch {}
  }

  throw new Error(
    `marisjs: Could not find platform package "${pkg}". ` +
    `Ensure the package is installed (npm install marisjs).`
  );
}

const binary = findBinary();

if (!existsSync(binary)) {
  console.error(
    `marisjs: Binary not found at "${binary}". ` +
    `The platform package was installed but the binary file is missing. ` +
    `Try reinstalling: npm install marisjs`
  );
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
process.exit(result.status ?? 1);
