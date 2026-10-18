//! 任务状态持久化：队列快照 + 异常中断检测（崩溃恢复）
//!
//! 两个文件都放在 `crate::dirs::load_dirs().config` 下：
//! - `任务快照.json`   ：任务列表 / 队列运行状态 / 保存时间（UTF-8 无 BOM，pretty JSON）
//! - `任务快照.running`：本次运行标记（空文件，正常退出时由 `mark_clean_exit` 删除）
//!
//! 判断逻辑：启动时若 `任务快照.running` 仍然存在，说明上次没有走正常退出流程
//! （崩溃 / 任务管理器强杀 / 断电），`load_queue_snapshot` 会把快照标记为
//! `interrupted`，并把所有 `running` 任务降级为 `pending` —— 进程已经没了，
//! 任务不可能还在跑，界面可以放心地把它当成「等待中」重新入队。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 快照文件名
pub const SNAPSHOT_FILE: &str = "任务快照.json";
/// 本次运行标记文件名
pub const RUNNING_MARK: &str = "任务快照.running";
/// 恢复摘要里最多展示的任务名数量
const MAX_SUMMARY_NAMES: usize = 5;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TaskSnapshot {
    pub id: String,
    pub name: String,
    /// "pending" | "running" | "paused" | "done" | "failed"
    pub status: String,
    pub progress: f64,
    /// 阶段："download" | "verify" | "install" | "done" | "error"
    pub stage: String,
    pub message: String,
    /// 已下载字节数（断点续传依据）
    #[serde(default)]
    pub downloaded: u64,
    /// 总字节数，0 表示未知
    #[serde(default)]
    pub total: u64,
    /// 安装包缓存路径
    #[serde(default)]
    pub file: String,
    /// 失败原因 / 返回码
    #[serde(default)]
    pub code: String,
    /// 最后更新时间戳（Unix 秒）
    pub updated: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct QueueSnapshot {
    #[serde(default)]
    pub tasks: Vec<TaskSnapshot>,
    /// 队列整体是否在运行中
    #[serde(default)]
    pub running: bool,
    /// 快照保存时间
    #[serde(default)]
    pub saved_at: i64,
    /// 是否是一次异常中断（程序上次没正常退出）
    #[serde(default)]
    pub interrupted: bool,
}

// ---------------------------------------------------------------------------
// 路径 / 读写
// ---------------------------------------------------------------------------

fn config_dir() -> PathBuf {
    PathBuf::from(crate::dirs::load_dirs().config)
}

fn snapshot_path() -> PathBuf {
    config_dir().join(SNAPSHOT_FILE)
}

fn marker_path() -> PathBuf {
    config_dir().join(RUNNING_MARK)
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 读取快照（文件缺失 / JSON 损坏都返回 None，调用方回退默认值）
fn read_snapshot() -> Option<QueueSnapshot> {
    let bytes = std::fs::read(snapshot_path()).ok()?;
    let text = crate::common::strip_bom(String::from_utf8_lossy(&bytes).into_owned());
    serde_json::from_str::<QueueSnapshot>(&text).ok()
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

/// 保存队列快照：自动填 `saved_at`，并落下本次运行标记
#[tauri::command]
pub fn save_queue_snapshot(mut snapshot: QueueSnapshot) -> Result<(), String> {
    snapshot.saved_at = now_ts();
    snapshot.interrupted = false;

    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    let text = serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;
    // UTF-8 无 BOM：直接写字符串
    std::fs::write(dir.join(SNAPSHOT_FILE), text.as_bytes())
        .map_err(|e| format!("写入任务快照失败: {}", e))?;
    // 运行标记：文件存在 = 本次运行还没正常收尾
    std::fs::write(dir.join(RUNNING_MARK), b"").map_err(|e| format!("写入运行标记失败: {}", e))?;
    Ok(())
}

/// 读取队列快照；不存在返回默认值。存在运行标记则判定为异常中断。
#[tauri::command]
pub fn load_queue_snapshot() -> QueueSnapshot {
    let Some(mut snap) = read_snapshot() else {
        return QueueSnapshot::default();
    };
    if marker_path().exists() {
        snap.interrupted = true;
        // 进程都不在了，队列不可能还在跑
        snap.running = false;
        snap.tasks = normalize_interrupted(std::mem::take(&mut snap.tasks));
    }
    snap
}

/// 正常退出：删除运行标记（父代理在窗口关闭 / 应用退出时调用）
#[tauri::command]
pub fn mark_clean_exit() -> Result<(), String> {
    match std::fs::remove_file(marker_path()) {
        Ok(()) => Ok(()),
        // 本来就没有标记 = 已是干净状态
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除运行标记失败: {}", e)),
    }
}

/// 清空快照与运行标记（用户放弃恢复 / 队列全部结束后调用）
#[tauri::command]
pub fn clear_queue_snapshot() -> Result<(), String> {
    for p in [snapshot_path(), marker_path()] {
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("删除 {} 失败: {}", p.display(), e)),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 纯函数（无 IO，便于单测）
// ---------------------------------------------------------------------------

/// 把快照里 running 的任务重置为 pending（异常中断后调用）。
/// done / failed / paused 等状态原样保留。
pub fn normalize_interrupted(tasks: Vec<TaskSnapshot>) -> Vec<TaskSnapshot> {
    tasks
        .into_iter()
        .map(|mut t| {
            if t.status == "running" {
                t.status = "pending".to_string();
            }
            t
        })
        .collect()
}

/// 判断快照是否值得恢复（有 pending / running 任务才需要）
pub fn has_resumable(snapshot: &QueueSnapshot) -> bool {
    snapshot
        .tasks
        .iter()
        .any(|t| t.status == "pending" || t.status == "running")
}

/// 生成给界面看的恢复摘要
/// 返回 `{ count, names: [前 5 个任务名], can_resume: bool }`
pub fn recovery_summary(snapshot: &QueueSnapshot) -> serde_json::Value {
    let resumable: Vec<&TaskSnapshot> = snapshot
        .tasks
        .iter()
        .filter(|t| t.status == "pending" || t.status == "running")
        .collect();
    let names: Vec<&str> = resumable
        .iter()
        .take(MAX_SUMMARY_NAMES)
        .map(|t| t.name.as_str())
        .collect();
    serde_json::json!({
        "count": resumable.len(),
        "names": names,
        "can_resume": has_resumable(snapshot),
    })
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, name: &str, status: &str) -> TaskSnapshot {
        TaskSnapshot {
            id: id.to_string(),
            name: name.to_string(),
            status: status.to_string(),
            progress: 0.0,
            stage: "download".to_string(),
            message: String::new(),
            downloaded: 0,
            total: 0,
            file: String::new(),
            code: String::new(),
            updated: 0,
        }
    }

    #[test]
    fn running_becomes_pending_after_crash() {
        let snap = QueueSnapshot {
            tasks: vec![
                task("a", "A", "running"),
                task("b", "B", "done"),
                task("c", "C", "failed"),
                task("d", "D", "pending"),
            ],
            running: true,
            saved_at: 100,
            interrupted: true,
        };

        let out = normalize_interrupted(snap.tasks);

        assert_eq!(out[0].status, "pending", "running 应降级为 pending");
        assert_eq!(out[1].status, "done", "done 不应被改动");
        assert_eq!(out[2].status, "failed", "failed 不应被改动");
        assert_eq!(out[3].status, "pending", "pending 保持 pending");
        // 其它字段不丢
        assert_eq!(out[0].id, "a");
        assert_eq!(out[0].name, "A");
    }

    #[test]
    fn has_resumable_detects_pending() {
        let finished = QueueSnapshot {
            tasks: vec![task("a", "A", "done"), task("b", "B", "failed")],
            ..Default::default()
        };
        assert!(!has_resumable(&finished));

        let mut with_pending = finished.clone();
        with_pending.tasks.push(task("c", "C", "pending"));
        assert!(has_resumable(&with_pending));

        let only_running = QueueSnapshot {
            tasks: vec![task("d", "D", "running")],
            ..Default::default()
        };
        assert!(has_resumable(&only_running));
    }

    #[test]
    fn summary_limits_names_to_five() {
        let tasks: Vec<TaskSnapshot> = (1..=10)
            .map(|i| task(&format!("id-{}", i), &format!("任务 {}", i), "pending"))
            .collect();
        let snap = QueueSnapshot { tasks, ..Default::default() };

        let s = recovery_summary(&snap);
        let names = s.get("names").and_then(|v| v.as_array()).expect("names 应为数组");

        assert_eq!(s.get("count").and_then(|v| v.as_u64()), Some(10));
        assert_eq!(names.len(), 5, "names 只取前 5 个");
        assert_eq!(names[0].as_str(), Some("任务 1"));
        assert_eq!(names[4].as_str(), Some("任务 5"));
        assert_eq!(s.get("can_resume").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn snapshot_json_roundtrip() {
        let mut file_task = task("a", "A", "running");
        file_task.progress = 42.5;
        file_task.stage = "verify".to_string();
        file_task.message = "校验中".to_string();
        file_task.downloaded = 4096;
        file_task.total = 8192;
        file_task.file = r"C:\cache\downloads\a.exe".to_string();
        file_task.code = "0".to_string();
        file_task.updated = 1_700_000_000;

        let snap = QueueSnapshot {
            tasks: vec![file_task],
            running: true,
            saved_at: 1_700_000_100,
            interrupted: true,
        };

        let text = serde_json::to_string(&snap).expect("序列化应成功");
        let back: QueueSnapshot = serde_json::from_str(&text).expect("反序列化应成功");

        assert!(back.running);
        assert_eq!(back.saved_at, 1_700_000_100);
        assert!(back.interrupted);
        assert_eq!(back.tasks.len(), 1);
        let t = &back.tasks[0];
        assert_eq!(t.id, "a");
        assert_eq!(t.name, "A");
        assert_eq!(t.status, "running");
        assert_eq!(t.progress, 42.5);
        assert_eq!(t.stage, "verify");
        assert_eq!(t.message, "校验中");
        assert_eq!(t.downloaded, 4096);
        assert_eq!(t.total, 8192);
        assert_eq!(t.file, r"C:\cache\downloads\a.exe");
        assert_eq!(t.code, "0");
        assert_eq!(t.updated, 1_700_000_000);

        // 缺省字段：老版本 / 手写 JSON 没有 downloaded / total / file / code
        let legacy = r#"{"id":"x","name":"X","status":"pending","progress":0.0,"stage":"download","message":"","updated":1}"#;
        let t: TaskSnapshot = serde_json::from_str(legacy).expect("缺省字段应可反序列化");
        assert_eq!(t.downloaded, 0);
        assert_eq!(t.total, 0);
        assert!(t.file.is_empty());
        assert!(t.code.is_empty());

        // QueueSnapshot 整体也可缺省
        let empty: QueueSnapshot = serde_json::from_str("{}").expect("空对象应可反序列化");
        assert!(empty.tasks.is_empty());
        assert!(!empty.running);
        assert!(!empty.interrupted);
    }

    #[test]
    fn empty_snapshot_is_not_resumable() {
        let snap = QueueSnapshot::default();
        assert!(!has_resumable(&snap));

        let s = recovery_summary(&snap);
        assert_eq!(s.get("count").and_then(|v| v.as_u64()), Some(0));
        assert_eq!(s.get("can_resume").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(s.get("names").and_then(|v| v.as_array()).map(|a| a.len()), Some(0));
    }
}
