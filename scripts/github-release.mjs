#!/usr/bin/env node
/**
 * github-release.mjs — create a GitHub release and upload asset files.
 *
 * Usage:
 *   GH_TOKEN=<token> node scripts/github-release.mjs \
 *     --repo owner/repo --tag v1.0.0 --name "v1.0.0" \
 *     [--notes-file notes.md] [--prerelease] \
 *     --assets "path/to/a.dmg" "path/to/b.zip"
 */
import { readFileSync, statSync } from "node:fs";

const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const has = (name) => args.includes(name);

const token = process.env.GH_TOKEN;
if (!token) {
  console.error("GH_TOKEN is required");
  process.exit(1);
}
const repo = opt("--repo");
const tag = opt("--tag");
const name = opt("--name") ?? tag;
const notesFile = opt("--notes-file");
if (!repo || !tag) {
  console.error("--repo and --tag are required");
  process.exit(1);
}
const assetIdx = args.indexOf("--assets");
const assets = assetIdx >= 0 ? args.slice(assetIdx + 1) : [];
const prerelease = has("--prerelease");

const headers = {
  Authorization: `Bearer ${token}`,
  Accept: "application/vnd.github+json",
};

async function main() {
  const body = notesFile ? readFileSync(notesFile, "utf8") : "";
  const payload = { tag_name: tag, name, body, prerelease, draft: false };

  const res = await fetch(`https://api.github.com/repos/${repo}/releases`, {
    method: "POST",
    headers: { ...headers, "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
  const release = await res.json();
  if (!res.ok || !release.id) {
    console.error("create release failed:", release.message ?? release);
    process.exit(1);
  }
  console.log(`release created: ${release.html_url}`);

  for (const file of assets) {
    const base = file.split("/").pop();
    const stat = statSync(file);
    const up = await fetch(
      `${release.upload_url.replace("{?name,label}", "")}?name=${encodeURIComponent(base)}`,
      {
        method: "POST",
        headers: {
          ...headers,
          "Content-Type": "application/octet-stream",
          "Content-Length": String(stat.size),
        },
        body: readFileSync(file),
      }
    );
    const asset = await up.json();
    if (!up.ok || !asset.id) {
      console.error(`upload ${base} failed:`, asset.message ?? asset);
      process.exit(1);
    }
    console.log(`asset uploaded: ${base} (${asset.size} bytes) -> ${asset.browser_download_url}`);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
