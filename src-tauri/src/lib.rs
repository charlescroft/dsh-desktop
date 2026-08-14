//! DSH Desktop — a thin Tauri shell around the DeepSeek Harness web profile.
//!
//! Responsibilities:
//! - resolve a self-contained Node + npm (sidecar / bundle resources), or the
//!   system `node`/`npm` in dev builds
//! - keep a per-user "runtime" npm project (app data dir) whose only
//!   dependency is `@deepseek-ai/dsh`, installed / upgraded on demand
//! - spawn `dsh --profile web --port 0`, parse the printed URL, and point the
//!   WebView at it; stop the child when the app quits
//! - native menu: check for updates, restart service, open in browser, open
//!   data/runtime/log dirs
//! - splash-page IPC (`boot_status`) so first-run installs show progress

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, RunEvent, State, Url, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

const DSH_PKG: &str = "@deepseek-ai/dsh";
const RUNTIME_PACKAGE_JSON: &str = r#"{
  "name": "dsh-desktop-runtime",
  "private": true,
  "dependencies": {
    "@deepseek-ai/dsh": "latest"
  }
}
"#;

/// True when the system locale is Chinese (zh-*).
fn zh_locale() -> bool {
    sys_locale::get_locale()
        .map(|l| l.to_lowercase().starts_with("zh"))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Paths {
    pub app_data: PathBuf,
    pub runtime: PathBuf,
    pub logs: PathBuf,
    pub dsh_home: PathBuf,
    pub workspace: PathBuf,
    pub npm_cache: PathBuf,
}

impl Paths {
    fn resolve(app: &AppHandle) -> Self {
        let app_data = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        let home = app
            .path()
            .home_dir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| PathBuf::from("."));
        let dsh_home = std::env::var("DSH_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".dsh"));
        let workspace = std::env::var("DSH_DESKTOP_WORKSPACE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home);
        Self {
            app_data: app_data.clone(),
            runtime: app_data.join("runtime"),
            logs: app_data.join("logs"),
            dsh_home,
            workspace,
            npm_cache: app_data.join("npm-cache"),
        }
    }
}

// ---------------------------------------------------------------------------
// Platform helpers
// ---------------------------------------------------------------------------

/// Open a URL or folder with the platform's default handler.
#[cfg(target_os = "windows")]
fn open_external(target: &str) {
    let _ = Command::new("cmd")
        .args(["/C", "start", "", target])
        .spawn();
}

#[cfg(target_os = "macos")]
fn open_external(target: &str) {
    let _ = Command::new("/usr/bin/open").arg(target).spawn();
}

#[cfg(target_os = "linux")]
fn open_external(target: &str) {
    let _ = Command::new("xdg-open").arg(target).spawn();
}

/// Send a termination signal to a child process.
#[cfg(target_os = "windows")]
fn terminate(pid: u32) {
    // /T kills the whole tree (node + shells it spawned), /F force.
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status();
}

#[cfg(not(target_os = "windows"))]
fn terminate(pid: u32) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
}

/// The splash page URL of this app's own origin.
#[cfg(target_os = "windows")]
fn splash_url() -> &'static str {
    "http://tauri.localhost/index.html"
}

#[cfg(not(target_os = "windows"))]
fn splash_url() -> &'static str {
    "tauri://localhost/index.html"
}

/// PATH list separator for the current platform.
#[cfg_attr(debug_assertions, allow(dead_code))]
#[cfg(target_os = "windows")]
fn path_sep() -> char {
    ';'
}

#[cfg(not(target_os = "windows"))]
fn path_sep() -> char {
    ':'
}

/// Candidate sidecar file names, most specific first.
#[cfg_attr(debug_assertions, allow(dead_code))]
#[cfg(target_os = "windows")]
fn sidecar_names() -> Vec<String> {
    vec!["node-x86_64-pc-windows-msvc.exe".into(), "node.exe".into()]
}

#[cfg(target_os = "macos")]
fn sidecar_names() -> Vec<String> {
    vec![
        "node-aarch64-apple-darwin".into(),
        "node-x86_64-apple-darwin".into(),
        "node".into(),
    ]
}

#[cfg(target_os = "linux")]
fn sidecar_names() -> Vec<String> {
    vec![
        "node-x86_64-unknown-linux-gnu".into(),
        "node-aarch64-unknown-linux-gnu".into(),
        "node".into(),
    ]
}

// ---------------------------------------------------------------------------
// Boot status (splash page polling)
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "phase")]
pub enum BootStatus {
    Installing,
    Starting,
    Ready { url: String },
    Failed { message: String },
}

// ---------------------------------------------------------------------------
// Server manager
// ---------------------------------------------------------------------------

pub struct ServerManager {
    app: AppHandle,
    paths: Paths,
    zh: bool,
    child: Mutex<Option<Child>>,
    url: Mutex<Option<String>>,
    status: Mutex<BootStatus>,
}

impl ServerManager {
    pub fn new(app: AppHandle) -> Self {
        let paths = Paths::resolve(&app);
        fs::create_dir_all(&paths.logs).ok();
        Self {
            app,
            paths,
            zh: zh_locale(),
            child: Mutex::new(None),
            url: Mutex::new(None),
            status: Mutex::new(BootStatus::Starting),
        }
    }

    // -- logging ------------------------------------------------------------

    fn log(&self, msg: &str) {
        let file = self.paths.logs.join("app.log");
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&file) {
            let _ = writeln!(f, "[{ts}] {msg}");
        }
    }

    fn log_file(&self) -> PathBuf {
        self.paths.logs.join("server.log")
    }

    // -- binary resolution --------------------------------------------------

    /// The node binary: bundled sidecar in release, `node` from PATH in dev.
    fn node_bin(&self) -> Option<PathBuf> {
        #[cfg(debug_assertions)]
        {
            Some(PathBuf::from("node"))
        }
        #[cfg(not(debug_assertions))]
        {
            let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
            let res_dir = self.app.path().resource_dir().ok();
            let mut candidates: Vec<PathBuf> = sidecar_names()
                .iter()
                .map(|n| exe_dir.join(n))
                .collect();
            if let Some(r) = res_dir {
                for n in sidecar_names() {
                    candidates.push(r.join("binaries").join(n));
                }
            }
            candidates.into_iter().find(|c| c.exists())
        }
    }

    /// Build an `npm` invocation: bundled node + npm-cli.js in release,
    /// the `npm` executable in dev.
    fn npm_command(&self) -> Result<Command, String> {
        #[cfg(debug_assertions)]
        {
            #[cfg(target_os = "windows")]
            {
                Ok(Command::new("npm.cmd"))
            }
            #[cfg(not(target_os = "windows"))]
            {
                Ok(Command::new("npm"))
            }
        }
        #[cfg(not(debug_assertions))]
        {
            let node = self.node_bin().ok_or("bundled Node runtime not found")?;
            let res_dir = self
                .app
                .path()
                .resource_dir()
                .map_err(|e| format!("cannot resolve app resource dir: {e}"))?;
            let npm_cli = res_dir
                .join("nodedist")
                .join("npm")
                .join("bin")
                .join("npm-cli.js");
            if !npm_cli.exists() {
                return Err(format!("bundled npm is missing: {}", npm_cli.display()));
            }
            let mut cmd = Command::new(&node);
            cmd.arg(npm_cli);
            // package install scripts (e.g. koffi's `node ./cnoke.cjs`) need
            // `node` on PATH: prepend the bundled node's directory.
            if let Some(dir) = node.parent() {
                let path = std::env::var("PATH").unwrap_or_default();
                cmd.env(
                    "PATH",
                    format!("{}{}{}", dir.display(), path_sep(), path),
                );
            }
            Ok(cmd)
        }
    }

    // -- runtime install ----------------------------------------------------

    fn installed_version(&self) -> Option<String> {
        let p = self
            .paths
            .runtime
            .join("node_modules")
            .join(DSH_PKG)
            .join("package.json");
        let text = fs::read_to_string(p).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        v.get("version")?.as_str().map(String::from)
    }

    fn pin_runtime_version(&self) -> Result<(), String> {
        let v = self
            .installed_version()
            .ok_or("cannot read installed dsh version")?;
        let pkg = format!(
            "{{\n  \"name\": \"dsh-desktop-runtime\",\n  \"private\": true,\n  \"dependencies\": {{\n    \"{DSH_PKG}\": \"{v}\"\n  }}\n}}\n"
        );
        fs::write(self.paths.runtime.join("package.json"), pkg)
            .map_err(|e| e.to_string())
    }

    fn ensure_runtime(&self) -> Result<(), String> {
        let pkg_json = self.paths.runtime.join("package.json");
        if !pkg_json.exists() {
            fs::create_dir_all(&self.paths.runtime).map_err(|e| e.to_string())?;
            fs::write(&pkg_json, RUNTIME_PACKAGE_JSON).map_err(|e| e.to_string())?;
        }
        if self.installed_version().is_none() {
            self.set_status(BootStatus::Installing);
            self.log("installing runtime dependencies (first run)…");
            let out = self
                .npm_command()?
                .arg("install")
                .arg("--no-audit")
                .arg("--no-fund")
                .arg("--loglevel=error")
                .current_dir(&self.paths.runtime)
                .env("npm_config_cache", &self.paths.npm_cache)
                .env("npm_config_update_notifier", "false")
                .output()
                .map_err(|e| format!("cannot run npm install: {e}"))?;
            if !out.status.success() {
                let msg = String::from_utf8_lossy(&out.stderr);
                self.log(&format!("npm install failed: {msg}"));
                return Err(format!(
                    "runtime install failed (network required): {}",
                    msg.trim().lines().last().unwrap_or("unknown error")
                ));
            }
            self.pin_runtime_version()?;
            self.log("runtime ready");
        }
        Ok(())
    }

    // -- server lifecycle ---------------------------------------------------

    pub fn start_server(&self) -> Result<(), String> {
        self.ensure_runtime()?;

        let node = self.node_bin().ok_or("Node runtime not found")?;
        let bin_js = self
            .paths
            .runtime
            .join("node_modules")
            .join(DSH_PKG)
            .join("lib")
            .join("bin.js");
        if !bin_js.exists() {
            return Err(format!("dsh entry is missing: {}", bin_js.display()));
        }

        self.log("starting dsh web server…");
        let mut cmd = Command::new(&node);
        cmd.arg(&bin_js)
            .arg("--profile")
            .arg("web")
            .arg("--port")
            .arg("0")
            .arg("--host")
            .arg("127.0.0.1")
            .current_dir(&self.paths.workspace)
            .env("DSH_HOME", &self.paths.dsh_home)
            .env("DSH_DESKTOP", "1")
            .env("NODE_ENV", "production");
        #[cfg(not(target_os = "windows"))]
        {
            // GUI-launched apps get a minimal PATH; make the common
            // user toolchains (Homebrew, /usr/local) available to the
            // dsh server and the shell tools it spawns.
            cmd.env(
                "PATH",
                format!(
                    "/opt/homebrew/bin:/usr/local/bin:{}",
                    std::env::var("PATH")
                        .unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into())
                ),
            );
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("failed to start dsh service: {e}"))?;

        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let app = self.app.clone();
        let server_log = self.log_file();
        let server_log_err = server_log.clone();

        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                append_line(&server_log, &line);
                if let Some(idx) = line.find("dsh web: http://") {
                    let rest = &line[idx + "dsh web: http://".len()..];
                    let url = format!("http://{}", rest.trim());
                    let mgr = app.state::<ServerManager>();
                    *mgr.url.lock().unwrap() = Some(url.clone());
                    mgr.set_status(BootStatus::Ready { url: url.clone() });
                    mgr.log(&format!("server ready at {url}"));
                    let app2 = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        if let Some(w) = app2.get_webview_window("main") {
                            if let Ok(u) = Url::parse(&url) {
                                let _ = w.navigate(u);
                            }
                        }
                    });
                }
            }
        });

        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                append_line(&server_log_err, &line);
            }
        });

        *self.child.lock().unwrap() = Some(child);
        self.set_status(BootStatus::Starting);
        Ok(())
    }

    pub fn stop(&self) {
        let mut guard = self.child.lock().unwrap();
        if let Some(child) = guard.as_mut() {
            self.log("stopping dsh server");
            terminate(child.id());
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            loop {
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = child.kill();
            let _ = child.wait();
            self.log("dsh server stopped");
        }
        *guard = None;
        drop(guard);
        *self.url.lock().unwrap() = None;
    }

    pub fn restart(&self) {
        self.stop();
        // back to the splash while the new server boots
        if let Some(w) = self.app.get_webview_window("main") {
            if let Ok(u) = Url::parse(splash_url()) {
                let _ = w.navigate(u);
            }
        }
        self.set_status(BootStatus::Starting);
        match self.start_server() {
            Ok(()) => self.log("restarted"),
            Err(e) => {
                self.set_status(BootStatus::Failed { message: e.clone() });
                let msg = if self.zh {
                    format!("重启失败：{e}")
                } else {
                    format!("Restart failed: {e}")
                };
                self.notify_error(&msg);
            }
        }
    }

    // -- update -------------------------------------------------------------

    fn latest_version(&self) -> Result<String, String> {
        let node = self.node_bin().ok_or("Node runtime not found")?;
        let script = r#"fetch("https://registry.npmjs.org/@deepseek-ai/dsh/latest")
  .then(r => { if (!r.ok) throw new Error("HTTP " + r.status); return r.json(); })
  .then(j => console.log(j.version))
  .catch(e => { console.error(e); process.exit(1); })"#;
        let out = Command::new(&node)
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "{}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if v.is_empty() {
            Err("empty registry response".into())
        } else {
            Ok(v)
        }
    }

    fn apply_update(&self, version: &str) -> Result<(), String> {
        self.log(&format!("updating {DSH_PKG} to {version}…"));
        let out = self
            .npm_command()?
            .arg("install")
            .arg("--save-exact")
            .arg("--no-audit")
            .arg("--no-fund")
            .arg("--loglevel=error")
            .arg(format!("{DSH_PKG}@{version}"))
            .current_dir(&self.paths.runtime)
            .env("npm_config_cache", &self.paths.npm_cache)
            .env("npm_config_update_notifier", "false")
            .output()
            .map_err(|e| format!("cannot run npm install: {e}"))?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr);
            self.log(&format!("update failed: {msg}"));
            return Err(format!(
                "update failed: {}",
                msg.trim().lines().last().unwrap_or("unknown error")
            ));
        }
        self.pin_runtime_version()?;
        self.log(&format!("updated to {version}"));
        Ok(())
    }

    // -- helpers ------------------------------------------------------------

    fn set_status(&self, status: BootStatus) {
        *self.status.lock().unwrap() = status;
    }

    pub fn status(&self) -> BootStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn current_url(&self) -> Option<String> {
        self.url.lock().unwrap().clone()
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    fn notify_error(&self, message: &str) {
        self.app
            .dialog()
            .message(message)
            .title("DSH Desktop")
            .show(|_| {});
    }
}

fn append_line(file: &Path, line: &str) {
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "{line}");
    }
}

// ---------------------------------------------------------------------------
// Update orchestration
// ---------------------------------------------------------------------------

/// Ask the user about a newer version and install it (blocking install on a
/// worker thread, then restart the service).
fn check_update(app: AppHandle, silent: bool) {
    let mgr = app.state::<ServerManager>();
    let zh = mgr.zh;
    let current = mgr.installed_version();
    let latest = match mgr.latest_version() {
        Ok(v) => v,
        Err(e) => {
            if !silent {
                let msg = if zh {
                    format!("检查更新失败：{e}")
                } else {
                    format!("Update check failed: {e}")
                };
                mgr.notify_error(&msg);
            }
            return;
        }
    };
    match current {
        Some(cur) if cur == latest => {
            if !silent {
                let (msg, title) = if zh {
                    (format!("当前已是最新版本 v{cur}。"), "检查更新")
                } else {
                    (format!("Already up to date (v{cur})."), "Check for Updates")
                };
                app.dialog().message(msg).title(title).show(|_| {});
            }
        }
        Some(cur) => {
            let app2 = app.clone();
            let (body, title, ok_btn, cancel_btn, done_msg, done_title) = if zh {
                (
                    format!(
                        "DeepSeek Harness 有新版本：\n当前 v{cur} → 最新 v{latest}\n\n是否立即更新？更新完成后服务会自动重启。"
                    ),
                    "发现新版本",
                    "立即更新",
                    "稍后",
                    format!("已更新到 v{latest}，服务已重启。"),
                    "更新完成",
                )
            } else {
                (
                    format!(
                        "A new version of DeepSeek Harness is available:\nCurrent v{cur} → Latest v{latest}\n\nUpdate now? The service will restart automatically."
                    ),
                    "Update Available",
                    "Update Now",
                    "Later",
                    format!("Updated to v{latest}. Service restarted."),
                    "Update Complete",
                )
            };
            app.dialog()
                .message(body)
                .title(title)
                .buttons(MessageDialogButtons::OkCancelCustom(
                    ok_btn.into(),
                    cancel_btn.into(),
                ))
                .show(move |yes| {
                    if yes {
                        let app = app2.clone();
                        std::thread::spawn(move || {
                            let mgr = app.state::<ServerManager>();
                            match mgr.apply_update(&latest) {
                                Ok(()) => {
                                    mgr.restart();
                                    app.dialog()
                                        .message(done_msg)
                                        .title(done_title)
                                        .show(|_| {});
                                }
                                Err(e) => mgr.notify_error(&e),
                            }
                        });
                    }
                })
        }
        None => {
            // runtime not installed yet; nothing to compare
        }
    }
}

// ---------------------------------------------------------------------------
// Commands (splash page IPC)
// ---------------------------------------------------------------------------

#[tauri::command]
fn boot_status(state: State<'_, ServerManager>) -> BootStatus {
    state.status()
}

#[tauri::command]
fn open_in_browser(state: State<'_, ServerManager>) -> Result<(), String> {
    match state.current_url() {
        Some(url) => {
            open_external(&url);
            Ok(())
        }
        None => Err("service not ready yet".into()),
    }
}

// ---------------------------------------------------------------------------
// Menu
// ---------------------------------------------------------------------------

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let zh = zh_locale();
    let (about_text, quit_text, app_menu_title, service_title) = if zh {
        ("关于 DSH Desktop", "退出 DSH Desktop", "DSH Desktop", "服务")
    } else {
        ("About DSH Desktop", "Quit DSH Desktop", "DSH Desktop", "Service")
    };
    let (check_item, restart_item, open_browser_item, open_dsh_home, open_runtime, open_logs) =
        if zh {
            (
                "检查更新…",
                "重启服务",
                "在系统浏览器中打开",
                "打开数据目录 (~/.dsh)",
                "打开运行时目录",
                "打开日志目录",
            )
        } else {
            (
                "Check for Updates…",
                "Restart Service",
                "Open in System Browser",
                "Open Data Folder (~/.dsh)",
                "Open Runtime Folder",
                "Open Logs Folder",
            )
        };
    let about = PredefinedMenuItem::about(app, Some(about_text), None)?;
    let quit = PredefinedMenuItem::quit(app, Some(quit_text))?;
    let app_menu = Submenu::with_items(
        app,
        app_menu_title,
        true,
        &[&about, &PredefinedMenuItem::separator(app)?, &quit],
    )?;
    let service = Submenu::with_items(
        app,
        service_title,
        true,
        &[
            &MenuItem::with_id(app, "check-update", check_item, true, None::<&str>)?,
            &MenuItem::with_id(app, "restart", restart_item, true, None::<&str>)?,
            &MenuItem::with_id(app, "open-browser", open_browser_item, true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "open-dsh-home", open_dsh_home, true, None::<&str>)?,
            &MenuItem::with_id(app, "open-runtime", open_runtime, true, None::<&str>)?,
            &MenuItem::with_id(app, "open-logs", open_logs, true, None::<&str>)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &service])
}

fn open_folder(path: &Path) {
    open_external(&path.display().to_string());
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    let mgr = app.state::<ServerManager>();
    match id {
        "check-update" => {
            let app = app.clone();
            std::thread::spawn(move || check_update(app, false));
        }
        "restart" => mgr.restart(),
        "open-browser" => match mgr.current_url() {
            Some(url) => {
                open_external(&url);
            }
            None => {
                let msg = if mgr.zh {
                    "服务尚未就绪，请稍后再试。"
                } else {
                    "Service is not ready yet. Please try again later."
                };
                mgr.notify_error(msg);
            }
        },
        "open-dsh-home" => {
            let d = mgr.paths().dsh_home.clone();
            fs::create_dir_all(&d).ok();
            open_folder(&d);
        }
        "open-runtime" => {
            let d = mgr.paths().runtime.clone();
            fs::create_dir_all(&d).ok();
            open_folder(&d);
        }
        "open-logs" => {
            let d = mgr.paths().logs.clone();
            fs::create_dir_all(&d).ok();
            open_folder(&d);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .menu(build_menu)
        .on_menu_event(|app, event| {
            handle_menu_event(app, event.id().as_ref());
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { .. } = event {
                let app = window.app_handle();
                app.state::<ServerManager>().stop();
                app.exit(0);
            }
        })
        .setup(|app| {
            let app = app.handle().clone();
            let mgr = ServerManager::new(app.clone());
            app.manage(mgr);

            let app_start = app.clone();
            let app_update = app.clone();
            std::thread::spawn(move || match app_start.state::<ServerManager>().start_server() {
                Ok(()) => {}
                Err(e) => {
                    app_start
                        .state::<ServerManager>()
                        .set_status(BootStatus::Failed { message: e.clone() });
                    let msg = if zh_locale() {
                        format!("启动失败：{e}\n\n详情见“服务 → 打开日志目录”。")
                    } else {
                        format!("Startup failed: {e}\n\nSee Service → Open Logs Folder for details.")
                    };
                    app_start
                        .dialog()
                        .message(msg)
                        .title("DSH Desktop")
                        .show(|_| {});
                }
            });

            // silent auto-update check shortly after boot
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(10));
                check_update(app_update, true);
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![boot_status, open_in_browser])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::ExitRequested { .. } = event {
                app.state::<ServerManager>().stop();
            }
        });
}
