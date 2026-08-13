# DSH Desktop

> [English](README.md)

用 **Tauri 2** 把 [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness)（`dsh`）的 Web 版打包成 macOS 原生桌面应用——不需要终端，也不需要浏览器标签页。

解决日常使用的三个痛点：

1. **不用再手动 `npx`** —— 双击 App 即自动安装/启动运行时并拉起服务；
2. **不用依赖浏览器** —— Web 界面渲染在原生窗口里（可缩放、可最大化）；
3. **独立应用** —— Dock 图标、原生菜单栏，服务进程随应用启停。

## 架构

```
┌────────────────────────── DSH Desktop.app ──────────────────────────┐
│  Rust 壳 (src-tauri)                                                │
│  ├─ 原生菜单：检查更新 / 重启服务 / 浏览器打开 / 打开目录             │
│  ├─ 进程管理：spawn/kill dsh server（退出应用自动停进程）             │
│  ├─ 内置 Node 24 (sidecar) + 内置 npm (资源)                        │
│  └─ WebView：先显示启动页，服务就绪后导航到 dsh UI                   │
└──────────────────────────────────────────────────────────────────────┘
        │ spawn: node bin.js --profile web --port 0 --host 127.0.0.1
        │ stdout 打印 "dsh web: http://127.0.0.1:<port>" → 解析 URL → 导航
        ▼
┌─────────────────── 运行时目录（用户可写、可升级）────────────────────┐
│  ~/Library/Application Support/com.deepseek.dshdesktop/runtime/     │
│  ├─ package.json           依赖: @deepseek-ai/dsh（精确锁定版本）     │
│  └─ node_modules/          dsh 核心及其全部依赖                       │
└──────────────────────────────────────────────────────────────────────┘
        │ DSH_HOME 不变
        ▼
  ~/.dsh  （你的全部会话 / 配置 / profile 原样沿用）
```

关键点：

- **复用基座一切能力**：桌面壳只是把 `dsh web` 进程的 stdout URL 接给 WebView，
  不做任何二次实现，dsh 升级即能力升级。
- **端口不冲突**：用 `--port 0` 让系统随机分配端口，与手动 `npx` 实例互不干扰。
- **可升级**：dsh 核心通过内置 npm 在线升级（见下文）；Tauri 壳保持极薄，
  重新 `npm run build` 即可出新版。

## 使用

- 双击 `DSH Desktop.app` 启动。首次运行会显示"正在安装运行时依赖"
  （需要网络，约 1 分钟），之后每次启动约 1–2 秒出界面。
- 应用图标与启动页使用 DeepSeek 官方鲸鱼标识（取自
  `@deepseek-ai/dsh-web-frontend` 的 favicon，MIT 许可），
  品牌蓝 `#4d6bfe`；启动页为深色模式下的纯鲸鱼。
- 界面语言跟随系统语言自动切换（中文 / English）。
- 菜单栏「服务」：
  - **检查更新…** 查询 npm registry 上的最新 `@deepseek-ai/dsh`，
    确认后在线安装并自动重启服务；
  - **重启服务** 手动重启（换 workspace 后可能需要）；
  - **在系统浏览器中打开** 在 Safari/Chrome 里打开同一实例
    （下载附件、文件另存等浏览器能力不足的场景可用）；
  - **打开数据目录 (~/.dsh)**、**打开运行时目录**、**打开日志目录**。
- 启动后 10 秒会自动静默检查一次更新，发现新版本会弹窗询问。

## 数据与日志

| 内容 | 位置 |
| --- | --- |
| 会话 / 配置 / profile | `~/.dsh`（与手动 npx 完全一致） |
| 运行时（dsh 核心） | `~/Library/Application Support/com.deepseek.dshdesktop/runtime/` |
| 日志 | `~/Library/Application Support/com.deepseek.dshdesktop/logs/` |

## 从源码构建

前置：macOS (Apple Silicon)、Xcode、[Rust](https://rustup.rs)、Node.js ≥ 20。

```bash
npm install            # 安装 @tauri-apps/cli、sharp
npm run fetch:node     # 下载官方 Node 24 (arm64)，产出 sidecar + 内置 npm
npm run icon           # 由 scripts/app-icon.png 生成全套图标（可先改图）
npm run build          # 产出 src-tauri/target/release/bundle/ 下的 .app 与 .dmg
```

开发模式（使用系统 node/npm，改 Rust 即时生效）：

```bash
npm run dev
```

## 升级路径

### 1. dsh 核心升级（日常，秒级）

「服务 → 检查更新…」，或接受启动后的自动提醒。内部等价于：

```bash
cd <runtime 目录> && npm install --save-exact @deepseek-ai/dsh@<新版>
```

更新后服务自动重启，无需重装 App。

### 2. 壳升级（少见）

壳（Rust/WKWebView 部分）变更时重新 `npm run build`。
如需全自动分发，可后续接入 [Tauri Updater](https://tauri.app/plugin/updater/)
（签名 + 静态文件托管，界面仍由 dsh 自身提供，不受影响）。

## 发行版

预编译产物挂在 [Releases](https://github.com/charlescroft/dsh-desktop/releases) 页面：
`.dmg` 安装包与 `.app` 压缩包。

## 已知限制

- 仅支持 macOS arm64（Apple Silicon）；其他平台需扩展 `fetch-node` 与二进制解析。
- WKWebView 不支持浏览器式"下载附件"行为：需要保存文件时用
  「在系统浏览器中打开」。
- 若 dsh 服务器崩溃，应用会停在启动页；日志见「服务 → 打开日志目录」。
- 同一时间建议只运行一个实例（所有实例共用 `~/.dsh`）。

## 目录结构

```
├── package.json          构建脚本（tauri CLI）
├── scripts/
│   ├── fetch-node.mjs    下载/暂存 Node 24 sidecar 与内置 npm
│   ├── make-icon.mjs     生成 1024px 应用图标
│   ├── whale.svg         官方 DeepSeek 鲸鱼标识（来自 dsh-web-frontend，MIT）
│   └── github-release.mjs 创建 GitHub release 并上传产物（GH_TOKEN）
├── ui/index.html         启动页（中英双语）
└── src-tauri/
    ├── tauri.conf.json   Tauri 配置（窗口 / 打包 / 资源）
    ├── src/lib.rs        Rust 壳：服务管理、菜单、升级、IPC
    ├── binaries/         Node sidecar（fetch-node 产出，不入库）
    └── nodedist/npm/     内置 npm（fetch-node 产出，不入库）
```

## 许可证

[MIT](LICENSE)
