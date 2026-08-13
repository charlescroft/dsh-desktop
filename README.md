# DSH Desktop

> [中文版](README.zh.md)

A native macOS desktop app that wraps the [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) (`dsh`) web profile in a **Tauri 2** shell — no terminal, no browser tab.

It solves three everyday annoyances:

1. **No more manual `npx`** — double-click the app; the runtime is installed and the service is started for you.
2. **No browser required** — the dsh UI renders in a native resizable window (WKWebView).
3. **A real app** — Dock icon, native menu bar, and the server process starts/stops with the app.

## How it works

```
┌────────────────────────── DSH Desktop.app ──────────────────────────┐
│  Rust shell (src-tauri)                                             │
│  ├─ Native menu: check updates / restart / open in browser / dirs   │
│  ├─ Process mgmt: spawn & kill the dsh server on app quit           │
│  ├─ Bundled Node 24 (sidecar) + bundled npm (resource)              │
│  └─ WebView: splash first, then navigate to the dsh UI              │
└──────────────────────────────────────────────────────────────────────┘
        │ spawn: node bin.js --profile web --port 0 --host 127.0.0.1
        │ stdout "dsh web: http://127.0.0.1:<port>" → parse → navigate
        ▼
┌─────────────── Runtime dir (user-writable, upgradable) ─────────────┐
│  ~/Library/Application Support/com.deepseek.dshdesktop/runtime/     │
│  ├─ package.json          dependency: @deepseek-ai/dsh (pinned)      │
│  └─ node_modules/         dsh core + all its dependencies            │
└──────────────────────────────────────────────────────────────────────┘
        │ DSH_HOME unchanged
        ▼
  ~/.dsh   (all your sessions / config / profiles, as before)
```

Key points:

- **Reuses the harness entirely**: the shell only hands the `dsh web` process's stdout URL to the WebView — no reimplementation, dsh upgrades are capability upgrades.
- **No port conflicts**: `--port 0` lets the OS pick a free port, so it never clashes with a manual `npx` instance.
- **Upgradable**: the dsh core is upgraded online via the bundled npm (see below); the Tauri shell stays thin, and a new version is just `npm run build` away.

## Usage

- Double-click `DSH Desktop.app`. First launch shows "installing runtime dependencies" (~1 min, network required); subsequent launches are ready in 1–2 s.
- Icons (app + splash) use the official DeepSeek whale mark (from `@deepseek-ai/dsh-web-frontend`, MIT), DeepSeek blue `#4d6bfe`; the splash shows the bare whale in dark mode.
- The UI language follows your system locale (中文 / English).
- Menu bar **Service**:
  - **Check for Updates…** — queries npm for the latest `@deepseek-ai/dsh`, installs it online and restarts the service;
  - **Restart Service** — manual restart (e.g. after switching workspace);
  - **Open in System Browser** — open the same instance in Safari/Chrome (handy for downloads WKWebView can't do);
  - **Open Data Folder (~/.dsh)**, **Open Runtime Folder**, **Open Logs Folder**.
- 10 s after launch a silent update check runs; it only prompts when a newer version exists.

## Data & logs

| What | Where |
| --- | --- |
| Sessions / config / profiles | `~/.dsh` (identical to manual npx usage) |
| Runtime (dsh core) | `~/Library/Application Support/com.deepseek.dshdesktop/runtime/` |
| Logs | `~/Library/Application Support/com.deepseek.dshdesktop/logs/` |

## Building from source

Prerequisites: macOS (Apple Silicon), Xcode, [Rust](https://rustup.rs), Node.js ≥ 20.

```bash
npm install            # @tauri-apps/cli, sharp
npm run fetch:node     # download official Node 24 (arm64) → sidecar + bundled npm
npm run icon           # regenerate all icon sizes from scripts/app-icon.png (edit the art first)
npm run build          # .app + .dmg in src-tauri/target/release/bundle/
```

Development mode (uses the system node/npm, hot-reloads Rust):

```bash
npm run dev
```

## Upgrading

### 1. dsh core (everyday, seconds)

Menu → Service → Check for Updates…, or accept the automatic prompt. Equivalent to:

```bash
cd <runtime dir> && npm install --save-exact @deepseek-ai/dsh@<new-version>
```

The service restarts automatically; no app reinstall needed.

### 2. The shell (rare)

Rebuild with `npm run build`. For fully automatic distribution, [Tauri Updater](https://tauri.app/plugin/updater/) can be added later (the UI itself is served by dsh, so it is unaffected).

## Releases

Prebuilt bundles are attached to the [Releases](https://github.com/charlescroft/dsh-desktop/releases) page: the `.dmg` installer and the `.app` zip.

## Known limitations

- macOS arm64 only (Apple Silicon) for now; other platforms need `fetch-node` and binary-resolution extensions.
- WKWebView cannot "download" attachments like a browser: use Service → Open in System Browser when you need to save files.
- If the dsh server crashes, the app stays on the splash; see Service → Open Logs Folder.
- Run one instance at a time (all instances share `~/.dsh`).

## Repository layout

```
├── package.json          build scripts (tauri CLI)
├── scripts/
│   ├── fetch-node.mjs    stage Node 24 sidecar + bundled npm
│   ├── make-icon.mjs     render the 1024 px app icon
│   ├── whale.svg         official DeepSeek whale mark (from dsh-web-frontend, MIT)
│   └── github-release.mjs create a GitHub release and upload assets (GH_TOKEN)
├── ui/index.html         splash page (i18n: zh / en)
└── src-tauri/
    ├── tauri.conf.json   Tauri config (window / bundling / resources)
    ├── src/lib.rs        Rust shell: service mgmt, menu, updates, IPC
    ├── binaries/         Node sidecar (produced by fetch-node, gitignored)
    └── nodedist/npm/     bundled npm (produced by fetch-node, gitignored)
```

## License

[MIT](LICENSE)
