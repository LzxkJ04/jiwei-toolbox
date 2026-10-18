//! 任务选项：队列执行模式（serial / parallel）、并发数与全局下载限速
//!
//! settings.json 中 `task_opts` 对象：
//! `{ "mode": "serial"|"parallel", "concurrency": 1~5, "speed_limit_kb": 0=不限 }`

use serde::Serialize;

/// 任务选项。字段保持 snake_case：前端 TaskOpts.speed_limit_kb 与 settings.json 都用这个名字。
#[derive(Debug, Clone, Serialize)]
pub struct TaskOpts {
    /// "serial" | "parallel"
    pub mode: String,
    /// 并行下载路数 1~5（serial 模式恒为 1）
    pub concurrency: u32,
    /// 全局限速 KB/s，0 = 不限速
    pub speed_limit_kb: u32,
}

impl Default for TaskOpts {
    fn default() -> Self {
        TaskOpts { mode: "serial".into(), concurrency: 1, speed_limit_kb: 0 }
    }
}

/// 读取任务选项（缺失或字段非法时逐项回退默认值）
pub fn load_task_opts() -> TaskOpts {
    let Some(v) = crate::common::read_setting_raw("task_opts") else {
        return TaskOpts::default();
    };
    let mode = v.get("mode").and_then(|x| x.as_str()).unwrap_or("serial");
    let mode = if mode == "parallel" { "parallel".to_string() } else { "serial".to_string() };
    let concurrency = v.get("concurrency").and_then(|x| x.as_u64()).unwrap_or(1).clamp(1, 5) as u32;
    let speed_limit_kb = v.get("speed_limit_kb").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    TaskOpts { mode, concurrency, speed_limit_kb }
}

/// 获取任务选项
#[tauri::command]
pub fn get_task_opts() -> TaskOpts {
    load_task_opts()
}

/// 设置任务选项（校验后持久化 settings.json）
/// rename_all = snake_case：前端传入的就是 speed_limit_kb，默认 camelCase 会找不到字段导致整次保存失败。
#[tauri::command(rename_all = "snake_case")]
pub fn set_task_opts(mode: String, concurrency: u32, speed_limit_kb: u32) -> Result<(), String> {
    if mode != "serial" && mode != "parallel" {
        return Err(format!("未知的执行模式: {}（仅支持 serial / parallel）", mode));
    }
    if !(1..=5).contains(&concurrency) {
        return Err("并发数必须在 1~5 之间".into());
    }
    if speed_limit_kb > 10_000_000 {
        return Err("限速值过大（上限 10000000 KB/s）".into());
    }
    crate::common::write_setting_raw(
        "task_opts",
        serde_json::json!({
            "mode": mode,
            "concurrency": concurrency,
            "speed_limit_kb": speed_limit_kb,
        }),
    )
}

// ---------------------------------------------------------------------------
// 共享限速器（100ms 窗口令牌桶，跨并发下载线程共享）
// ---------------------------------------------------------------------------

/// 每 100ms 释放一批预算的简单令牌桶：
/// 每个窗口允许 `per_window` 字节；线程消费 n 字节时若窗口预算不足则阻塞到下一窗口。
pub struct RateLimiter {
    /// 每 100ms 窗口允许的字节数（0 = 不限速）
    per_window: u64,
    /// (窗口开始时刻, 窗口内已消费字节)
    state: std::sync::Mutex<(std::time::Instant, u64)>,
}

impl RateLimiter {
    /// bytes_per_sec：每秒允许的字节数（speed_limit_kb * 1024）
    pub fn new(bytes_per_sec: u64) -> Self {
        RateLimiter {
            per_window: bytes_per_sec / 10,
            state: std::sync::Mutex::new((std::time::Instant::now(), 0)),
        }
    }

    /// 消费 n 字节预算，预算不足时阻塞等待（多次读块累计限速效果）
    pub fn consume(&self, n: u64) {
        if self.per_window == 0 || n == 0 {
            return;
        }
        // 单块超过整窗预算：按整窗数线性等待，余量按正常窗口入账
        if n > self.per_window {
            let full = n / self.per_window;
            let rem = n % self.per_window;
            std::thread::sleep(std::time::Duration::from_millis(full * 100));
            let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
            *g = (std::time::Instant::now(), rem);
            return;
        }
        loop {
            let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let (start, used) = *g;
            let now = std::time::Instant::now();
            let elapsed_ms = now.duration_since(start).as_millis() as u64;
            if elapsed_ms >= 100 {
                // 新窗口：预算重置
                *g = (now, n);
                return;
            }
            if used + n <= self.per_window {
                g.1 = used + n;
                return;
            }
            // 本窗口预算不足：睡到窗口结束（多线程在此自然排队）
            let wait = 100 - elapsed_ms + 1;
            drop(g);
            std::thread::sleep(std::time::Duration::from_millis(wait));
        }
    }
}
