#!/usr/bin/env node
/**
 * fetch-node.mjs — download the official Node.js distribution for the current
 * host platform and stage it for the Tauri bundle:
 *
 *   src-tauri/binaries/node-<target-triple>[.exe]   (node binary, externalBin sidecar)
 *   src-tauri/nodedist/npm/                          (npm-cli.js + its node_modules, bundle resource)
 *
 * Supported hosts: darwin-arm64/x64, win32-x64, linux-x64/arm64.
 * The pinned major is read from NODE_MAJOR (default 24 = active LTS line).
 */
import {
  createWriteStream,
  mkdirSync,
  rmSync,
  copyFileSync,
  cpSync,
  chmodSync,
  existsSync,
} from "node:fs";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";
import { fileURLToPath } from "node:url";
import path from "node:path";
import os from "node:os";
import { execFileSync } from "node:child_process";

const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const BIN_DIR = path.join(ROOT, "src-tauri", "binaries");
const NODE_DIST_DIR = path.join(ROOT, "src-tauri", "nodedist");
const NPM_DEST = path.join(NODE_DIST_DIR, "npm");

const MAJOR = process.env.NODE_MAJOR ?? "24";

// host key -> { dist, archive, bin, npm, sidecar name }
const PLATFORMS = {
  "darwin-arm64": {
    dist: "darwin-arm64",
    ext: "tar.gz",
    bin: "bin/node",
    npm: "lib/node_modules/npm",
    dest: "node-aarch64-apple-darwin",
  },
  "darwin-x64": {
    dist: "darwin-x64",
    ext: "tar.gz",
    bin: "bin/node",
    npm: "lib/node_modules/npm",
    dest: "node-x86_64-apple-darwin",
  },
  "win32-x64": {
    dist: "win-x64",
    ext: "zip",
    bin: "node.exe",
    npm: "node_modules/npm",
    dest: "node-x86_64-pc-windows-msvc.exe",
  },
  "linux-x64": {
    dist: "linux-x64",
    ext: "tar.gz",
    bin: "bin/node",
    npm: "lib/node_modules/npm",
    dest: "node-x86_64-unknown-linux-gnu",
  },
  "linux-arm64": {
    dist: "linux-arm64",
    ext: "tar.gz",
    bin: "bin/node",
    npm: "lib/node_modules/npm",
    dest: "node-aarch64-unknown-linux-gnu",
  },
};

const host = `${os.platform()}-${os.arch()}`;
const platform = PLATFORMS[host];
if (!platform) {
  console.error(`fetch-node: unsupported host ${host} (supported: ${Object.keys(PLATFORMS).join(", ")})`);
  process.exit(1);
}

const NODE_BIN_DEST = path.join(BIN_DIR, platform.dest);

async function latestVersion(major) {
  const res = await fetch("https://nodejs.org/dist/index.json");
  if (!res.ok) throw new Error(`registry index: HTTP ${res.status}`);
  const list = await res.json();
  const hit = list.find((v) => v.version.startsWith(`v${major}.`));
  if (!hit) throw new Error(`no release in the v${major}.x line`);
  return hit.version; // newest in that major line
}

async function main() {
  const version = await latestVersion(MAJOR);
  console.log(`fetch-node: staging Node ${version} for ${host}`);
  const bare = version.replace(/^v/, "");
  const archive = `node-v${bare}-${platform.dist}.${platform.ext}`;
  const url = `https://nodejs.org/dist/v${bare}/${archive}`;

  const tmp = path.join(os.tmpdir(), `node-${version}-${host}`);
  const archivePath = path.join(tmp, archive);
  mkdirSync(tmp, { recursive: true });

  if (!existsSync(archivePath)) {
    console.log(`fetch-node: downloading ${url} ...`);
    const res = await fetch(url);
    if (!res.ok) throw new Error(`download: HTTP ${res.status}`);
    await pipeline(Readable.fromWeb(res.body), createWriteStream(archivePath));
  }

  const extractDir = path.join(tmp, "x");
  rmSync(extractDir, { recursive: true, force: true });
  mkdirSync(extractDir, { recursive: true });
  // bsdtar on macOS/Windows handles zip as well; GNU tar handles tar.gz.
  execFileSync("tar", ["-xf", archivePath, "-C", extractDir, "--strip-components", "1"], {
    stdio: "inherit",
  });

  const nodeBin = path.join(extractDir, ...platform.bin.split("/"));
  const npmSrc = path.join(extractDir, ...platform.npm.split("/"));

  mkdirSync(BIN_DIR, { recursive: true });
  copyFileSync(nodeBin, NODE_BIN_DEST);
  if (os.platform() !== "win32") chmodSync(NODE_BIN_DEST, 0o755);

  mkdirSync(NODE_DIST_DIR, { recursive: true });
  rmSync(NPM_DEST, { recursive: true, force: true });
  cpSync(npmSrc, NPM_DEST, { recursive: true });

  rmSync(tmp, { recursive: true, force: true });
  console.log(`fetch-node: done -> ${NODE_BIN_DEST}`);
  console.log(`fetch-node: done -> ${NPM_DEST}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
