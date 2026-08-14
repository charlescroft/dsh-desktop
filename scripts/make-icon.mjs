#!/usr/bin/env node
/**
 * make-icon.mjs — render the DSH Desktop app icon (1024x1024 PNG) with sharp:
 * DeepSeek brand blue rounded square + the official whale mark (white),
 * taken from @deepseek-ai/dsh-web-frontend/dist/favicon.svg (MIT).
 * Then `npm run icon` runs `tauri icon` to derive .icns/.ico/png sizes.
 */
import sharp from "sharp";
import path from "node:path";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const OUT = path.join(ROOT, "scripts", "app-icon.png");

// DeepSeek brand blue
const BLUE = "#4d6bfe";

// macOS icon grid: the visible artwork must occupy ~80% of the canvas with
// ~10% transparent margins on every side (Apple's icon design spec), or the
// icon renders oversized in the Dock (opaque bounds hug the canvas edge).
const SIZE = 1024; // canvas
const GRID = Math.round(SIZE * 0.805); // 824: artwork tile
const GRID_MARGIN = Math.round((SIZE - GRID) / 2); // 100
const GRID_RADIUS = Math.round(GRID * 0.225); // Big Sur-style corner radius

// Whale mark: occupies ~71% of the artwork tile, centered.
const WHALE_SIZE = Math.round(GRID * 0.72); // 593

// Official whale mark: strip the dark-mode media query, force white, and
// render at the target size (viewBox stays 0 0 50 50, so coordinates hold).
let whale = readFileSync(path.join(ROOT, "scripts", "whale.svg"), "utf8");
whale = whale.replace(/<style>[\s\S]*?<\/style>/, "");
whale = whale.replace(/fill="#000"/, `fill="#fff"`);
whale = whale
  .replace(/width="50\.000000"/, `width="${WHALE_SIZE}"`)
  .replace(/height="50\.000000"/, `height="${WHALE_SIZE}"`);

const background = sharp({
  create: {
    width: SIZE,
    height: SIZE,
    channels: 4,
    background: { r: 0, g: 0, b: 0, alpha: 0 },
  },
})
  .composite([
    {
      input: Buffer.from(
        `<svg width="${SIZE}" height="${SIZE}" viewBox="0 0 ${SIZE} ${SIZE}" xmlns="http://www.w3.org/2000/svg">
          <defs>
            <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
              <stop offset="0" stop-color="${BLUE}"/>
              <stop offset="1" stop-color="#3b5bdb"/>
            </linearGradient>
          </defs>
          <rect x="${GRID_MARGIN}" y="${GRID_MARGIN}" width="${GRID}" height="${GRID}" rx="${GRID_RADIUS}" fill="url(#bg)"/>
        </svg>`
      ),
      top: 0,
      left: 0,
    },
    // whale mark centered in the artwork tile
    {
      input: Buffer.from(whale),
      top: Math.round((SIZE - WHALE_SIZE) / 2),
      left: Math.round((SIZE - WHALE_SIZE) / 2),
    },
  ]);

await background.png().toFile(OUT);
console.log(`make-icon: wrote ${OUT} (official whale mark on ${BLUE})`);
