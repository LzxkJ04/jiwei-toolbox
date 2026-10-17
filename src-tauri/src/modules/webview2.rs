//! WebView2 Evergreen 预检安装：下载独立安装器 + 静默安装（走系统代理）
//!
//! 事件：webview2-progress { stage: "download" | "install", pct }
//! 完成后前端可再次调用 env_status 复查 has_webview2。

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::Emitter;

/// 安装向导转发标记：置位时 webview2-progress 同步转发为
/// installer-progress {stage:"webview2", pct, msg}（供安装向导进度页展示）
static FORWARD_TO_INSTALLER: AtomicBool = AtomicBool::new(false);

pub fn set_forward_to_installer(v: bool) {
    FORWARD_TO_INSTALLER.store(v, Ordering::SeqCst);
}

/// Microsoft Edge WebView2 Evergreen 独立安装器（官方永久直链）
const WEBVIEW2_URL: &str = "https://go.microsoft.com/fwlink/?linkid=2124701";
/// 安装器本地文件名
const INSTALLER_NAME: &str = "MicrosoftEdgeWebView2RuntimeInstallerX64.exe";

/// 安装器保存路径：<cache>\webview2\<INSTALLER_NAME>
fn installer_path() -> PathBuf {
    let dirs = crate::dirs::load_dirs();
    PathBuf::from(&dirs.cache).join("webview2").join(INSTALLER_NAME)
}

fn emit_progress(app: &tauri::AppHandle, stage: &str, pct: i64) {
    let _ = app.emit("webview2-progress", serde_json::json!({ "stage": stage, "pct": pct }));
    if FORWARD_TO_INSTALLER.load(Ordering::SeqCst) {
        let _ = app.emit(
            "installer-progress",
            serde_json::json!({
                "stage": "webview2",
                "pct": pct,
                "msg": match stage {
                    "download" => "正在下载 WebView2 运行时…",
                    _ => "正在安装 WebView2 运行时…",
                }
            }),
        );
    }
}

/// 下载 WebView2 Evergreen 独立安装器到 cache（reqwest，默认启用系统代理环境变量），
/// 进度经 webview2-progress {stage:"download", pct} 推送。已存在则跳过直接返回。
#[tauri::command]
pub async fn download_webview2(app: tauri::AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || download_webview2_blocking(&app))
        .await
        .map_err(|e| format!("下载任务异常: {}", e))?
}

fn download_webview2_blocking(app: &tauri::AppHandle) -> Result<String, String> {    let dest = installer_path();
    if dest.exists() {
        emit_progress(app, "download", 100);
        return Ok(dest.to_string_lossy().into_owned());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建缓存目录失败: {}", e))?;
    }

    // reqwest 默认读取 HTTP_PROXY/HTTPS_PROXY 等环境变量（系统代理）
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("网络客户端初始化失败: {}", e))?;

    let mut resp = client
        .get(WEBVIEW2_URL)
        .send()
        .map_err(|e| format!("下载请求失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&dest).map_err(|e| format!("创建安装包文件失败: {}", e))?;
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_pct: i64 = -1;
    loop {
        let n = resp.read(&mut buf).map_err(|e| format!("下载中断: {}", e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| format!("写入安装包失败: {}", e))?;
        downloaded += n as u64;
        if total > 0 {
            let pct = (downloaded * 100 / total) as i64;
            if pct != last_pct {
                last_pct = pct;
                emit_progress(app, "download", pct);
            }
        }
    }
    file.flush().map_err(|e| format!("写入安装包失败: {}", e))?;
    emit_progress(app, "download", 100);
    Ok(dest.to_string_lossy().into_owned())
}

/// 静默安装 WebView2（/silent /install，阻塞等待完成）。
/// 完成后推送 webview2-progress {stage:"install", pct:100}；前端随后调用 env_status 复查。
#[tauri::command]
pub async fn install_webview2(app: tauri::AppHandle) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || install_webview2_blocking(&app))
        .await
        .map_err(|e| format!("安装任务异常: {}", e))?
}

/// 阻塞式「下载 + 静默安装」WebView2（供安装向导 inst_start 内部复用；
/// 进度经 set_forward_to_installer 转发为 installer-progress）
pub(crate) fn ensure_webview2_blocking(app: &tauri::AppHandle) -> Result<bool, String> {
    install_webview2_blocking(app)
}

fn install_webview2_blocking(app: &tauri::AppHandle) -> Result<bool, String> {
    // 未下载则先下载
    let dest = installer_path();
    if !dest.exists() {
        download_webview2_blocking(app)?;
    }
    if crate::common::has_webview2() {
        // 已安装：直接返回
        emit_progress(app, "install", 100);
        return Ok(true);
    }

    use std::os::windows::process::CommandExt;
    emit_progress(app, "install", 0);
    let status = std::process::Command::new(&dest)
        .args(["/silent", "/install"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status()
        .map_err(|e| format!("启动安装器失败: {}", e))?;
    if !status.success() {
        return Err(format!("WebView2 安装器异常退出: {:?}", status.code()));
    }
    emit_progress(app, "install", 100);

    let ok = crate::common::has_webview2();
    if let Some(parent) = dest.parent() {
        // 安装成功后清理安装包（失败忽略，供清理缓存兜底）
        if ok {
            let _ = std::fs::remove_file(&dest);
            let _ = std::fs::remove_dir(parent);
        }
    }
    Ok(ok)
}
