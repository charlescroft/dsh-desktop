#!/usr/bin/env node
/**
 * fetch-node.mjs — download the official Node.js distribution (macOS arm64)
 * and stage it for the Tauri bundle:
 *
 *   src-tauri/binaries/node-aarch64-apple-darwin   (the node binary, externalBin sidecar)
 *   src-tauri/nodedist/npm/                        (npm-cli.js + its node_modules, bundle resource)
 *
 * The pinned major is read from NODE_MAJOR (default 24 = active LTS line).
 */
import { createWriteStream, mkdirSync, rmSync, copyFileSync, chmodSync, existsSync } from "node:fs";
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
const NODE_BIN_DEST = path.join(BIN_DIR, "node-aarch64-apple-darwin");

const MAJOR = process.env.NODE_MAJOR ?? "24";

if (os.platform() !== "darwin" || os.arch() !== "arm64") {
  console.error(`fetch-node: only darwin-arm64 is supported right now (got ${os.platform()}/${os.arch()})`);
  process.exit(1);
}

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
  console.log(`fetch-node: staging Node ${version} (darwin-arm64)`);
  const bare = version.replace(/^v/, "");
  const tarball = `node-v${bare}-darwin-arm64.tar.gz`;
  const url = `https://nodejs.org/dist/v${bare}/${tarball}`;

  const tmp = path.join(os.tmpdir(), `node-${version}`);
  const tarballPath = path.join(tmp, tarball);
  mkdirSync(tmp, { recursive: true });
  rmSync(path.join(tmp, "extracted"), { recursive: true, force: true });

  if (!existsSync(tarballPath)) {
    console.log(`fetch-node: downloading ${url} ...`);
    const res = await fetch(url);
    if (!res.ok) throw new Error(`download: HTTP ${res.status}`);
    await pipeline(Readable.fromWeb(res.body), createWriteStream(tarballPath));
  }

  execFileSync("tar", ["-xzf", tarballPath, "-C", tmp, "--strip-components", "1"], { stdio: "inherit" });

  const nodeBin = path.join(tmp, "bin", "node");
  const npmSrc = path.join(tmp, "lib", "node_modules", "npm");

  mkdirSync(BIN_DIR, { recursive: true });
  copyFileSync(nodeBin, NODE_BIN_DEST);
  chmodSync(NODE_BIN_DEST, 0o755);

  mkdirSync(NODE_DIST_DIR, { recursive: true });
  rmSync(NPM_DEST, { recursive: true, force: true });
  execFileSync("cp", ["-R", npmSrc, NPM_DEST], { stdio: "inherit" });
  chmodSync(path.join(NPM_DEST, "bin", "npm-cli.js"), 0o755);

  rmSync(tmp, { recursive: true, force: true });
  console.log(`fetch-node: done -> ${NODE_BIN_DEST}`);
  console.log(`fetch-node: done -> ${NPM_DEST}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
