//! 定时自动备份任务（配置备份 .jwbak 的周期调度）
//!
//! ## 父代理接线（三处，缺一不可）
//!
//! 1. `src-tauri/src/modules/mod.rs` 增加一行：
//!    ```ignore
//!    pub mod schedule;
//!    ```
//! 2. `src-tauri/src/lib.rs` 的 `setup` 闭包里（主模式分支）调用一次：
//!    ```ignore
//!    schedule::start_scheduler();   // ← 或 modules::schedule::start_scheduler();
//!    ```
//! 3. `src-tauri/src/lib.rs` 的 `generate_handler![...]` 注册 7 个命令：
//!    ```ignore
//!    schedule::list_schedules,
//!    schedule::save_schedule,
//!    schedule::delete_schedule,
//!    schedule::toggle_schedule,
//!    schedule::run_schedule_now,
//!    schedule::cleanup_old_backups,
//!    schedule::get_schedule_status,
//!    ```
//!
//! ## 存储
//! 全部任务以数组形式存在 `config/settings.json` 的 `backup_schedules` 键下，
//! 读写走 `common::read_setting_raw` / `write_setting_raw`。
//!
//! ## 语义说明
//! `last_run` 同时充当"下次执行的基准时间"：任务从未执行过时，保存时会把
//! `last_run` 置为当前时间（避免一建任务就立刻跑一次），此时 `last_result`
//! 为空串，前端据此显示"从未执行"。

use chrono::{Datelike, Days, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use crate::modules::backup;
use crate::modules::common::{read_setting_raw, write_setting_raw};
use crate::modules::dirs::load_dirs;
use crate::modules::logs::op_log;

const SETTING_KEY: &str = "backup_schedules";
const BACKUP_EXT: &str = "jwbak";
const TICK_SECS: u64 = 60;

/* ============================ 数据结构 ============================ */

#[derive(Serialize, Deserialize, Clone)]
pub struct BackupTask {
    pub id: String,
    pub name: String,
    /// "daily" | "weekly" | "monthly"
    pub period: String,
    /// 触发时间 "HH:MM"（24 小时制）
    pub time: String,
    /// weekly 时为 1-7（1=周一）；monthly 时为 1-28；daily 忽略
    #[serde(default = "default_one")]
    pub day: u32,
    /// 要备份的软件 id 列表（来自软件库清单）；空表示全部已安装
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 保留最近 N 份，超过自动删旧的
    #[serde(default = "default_keep")]
    pub keep: u32,
    /// 上次执行时间戳（Unix 秒），0 表示从未执行
    #[serde(default)]
    pub last_run: i64,
    /// 上次执行结果
    #[serde(default)]
    pub last_result: String,
}

fn default_one() -> u32 {
    1
}
fn default_true() -> bool {
    true
}
fn default_keep() -> u32 {
    5
}

/* ============================ 时间计算（纯函数） ============================ */

/// 解析 "HH:MM"：非法（越界、非数字、多段、全角等）返回 None
fn parse_hhmm(time: &str) -> Option<(u32, u32)> {
    let t = time.trim();
    if t.is_empty() || !t.is_ascii() {
        return None;
    }
    let mut it = t.split(':');
    let hs = it.next()?;
    let ms = it.next()?;
    if it.next().is_some() {
        return None;
    }
    if hs.is_empty() || hs.len() > 2 || ms.len() != 2 {
        return None;
    }
    let h: u32 = hs.parse().ok()?;
    let m: u32 = ms.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

/// 某天的 h:m 对应的本地时间戳。
/// `from_local_datetime` 在夏令时重叠/跳空时返回 single()=None，此时视为不可用。
fn at_local(date: NaiveDate, h: u32, m: u32) -> Option<i64> {
    let naive = date.and_hms_opt(h, m, 0)?;
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|d| d.timestamp())
}

fn next_day(date: NaiveDate) -> Option<NaiveDate> {
    date.checked_add_days(Days::new(1))
}

fn add_days(date: NaiveDate, days: u64) -> Option<NaiveDate> {
    date.checked_add_days(Days::new(days))
}

fn next_month(y: i32, m: u32) -> (i32, u32) {
    if m >= 12 {
        (y + 1, 1)
    } else {
        (y, m + 1)
    }
}

/// 计算下一次应执行的时间戳（Unix 秒）。
/// now 为当前时间戳，task 为任务配置。返回 0 表示配置非法。
pub fn next_run_ts(now: i64, period: &str, time: &str, day: u32) -> i64 {
    let Some((h, m)) = parse_hhmm(time) else {
        return 0;
    };
    // 用 timestamp_opt 把时间戳落到本地时区，避免依赖 Local::now()
    let Some(base) = Local.timestamp_opt(now, 0).single() else {
        return 0;
    };
    let today = base.date_naive();

    match period {
        "daily" => {
            // 今天该时刻未过 → 今天；否则明天
            if let Some(ts) = at_local(today, h, m) {
                if ts > now {
                    return ts;
                }
            }
            next_day(today).and_then(|d| at_local(d, h, m)).unwrap_or(0)
        }
        "weekly" => {
            if !(1..=7).contains(&day) {
                return 0;
            }
            let cur = base.weekday().num_days_from_monday() + 1; // 1=周一..7=周日
            let ahead = (day + 7 - cur) % 7;
            let Some(target) = add_days(today, ahead as u64) else {
                return 0;
            };
            if let Some(ts) = at_local(target, h, m) {
                if ts > now {
                    return ts;
                }
            }
            // 今天就是该星期几但时刻已过 → 下周
            add_days(target, 7).and_then(|d| at_local(d, h, m)).unwrap_or(0)
        }
        "monthly" => {
            if !(1..=28).contains(&day) {
                return 0;
            }
            // day 限制在 1-28，闰年/月末都不会出现无效日期
            let (mut y, mut mo) = (today.year(), today.month());
            for _ in 0..=1 {
                if let Some(date) = NaiveDate::from_ymd_opt(y, mo, day) {
                    if let Some(ts) = at_local(date, h, m) {
                        if ts > now {
                            return ts;
                        }
                    }
                }
                let (ny, nm) = next_month(y, mo);
                y = ny;
                mo = nm;
            }
            0
        }
        _ => 0,
    }
}

/// 以 base 为基准，now 是否已经到达本应触发的时间点。
/// next_run_ts(base, ...) <= now 即认为需要触发；配置非法（0）不触发。
pub fn should_fire_at(now: i64, base: i64, period: &str, time: &str, day: u32) -> bool {
    let next = next_run_ts(base, period, time, day);
    next != 0 && next <= now
}

/// 判断任务此刻是否应该被触发（供调度线程使用）
pub fn should_fire(now: i64, task: &BackupTask) -> bool {
    task.enabled && should_fire_at(now, task.last_run, &task.period, &task.time, task.day)
}

/// 保留份数校验：1..=50
pub fn validate_keep(n: u32) -> Result<(), String> {
    if (1..=50).contains(&n) {
        Ok(())
    } else {
        Err(format!("保留份数需在 1-50 之间（当前 {}）", n))
    }
}

/* ============================ 配置读写 ============================ */

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

fn lock_cfg() -> std::sync::MutexGuard<'static, ()> {
    CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 读取任务列表（调用方需持有 CONFIG_LOCK）
fn read_tasks() -> Vec<BackupTask> {
    read_setting_raw(SETTING_KEY)
        .and_then(|v| serde_json::from_value::<Vec<BackupTask>>(v).ok())
        .map(|mut list| {
            // 容错：脏数据里可能缺 id，补一个稳定占位，避免前端 v-for 抖动
            for (i, t) in list.iter_mut().enumerate() {
                if t.id.trim().is_empty() {
                    t.id = format!("sch-imported-{}", i);
                }
                if t.keep == 0 {
                    t.keep = default_keep();
                }
            }
            list
        })
        .unwrap_or_default()
}

/// 写入任务列表（调用方需持有 CONFIG_LOCK）
fn write_tasks(list: &[BackupTask]) -> Result<(), String> {
    let v = serde_json::to_value(list).map_err(|e| e.to_string())?;
    write_setting_raw(SETTING_KEY, v)
}

fn load_tasks() -> Vec<BackupTask> {
    let _g = lock_cfg();
    read_tasks()
}

/// 只更新某个任务的 last_run / last_result（避免覆盖用户刚做的其它编辑）
fn update_task_result(id: &str, ts: i64, result: &str) -> Result<(), String> {
    let _g = lock_cfg();
    let mut list = read_tasks();
    let Some(t) = list.iter_mut().find(|x| x.id == id) else {
        return Ok(());
    };
    t.last_run = ts;
    t.last_result = result.to_string();
    write_tasks(&list)
}

/* ============================ 备份文件清理 ============================ */

/// 只清理 dir 下（不递归、不删子目录）的 .jwbak 文件，按修改时间倒序保留最新 keep 份
fn cleanup_in(dir: &str, keep: u32) -> Result<usize, String> {
    let d = Path::new(dir);
    if !d.is_dir() {
        return Ok(0);
    }
    let entries = std::fs::read_dir(d).map_err(|e| e.to_string())?;
    let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        if !p
            .extension()
            .map(|x| x.eq_ignore_ascii_case(BACKUP_EXT))
            .unwrap_or(false)
        {
            continue;
        }
        let mt = e
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        files.push((p, mt));
    }
    // 新 → 旧（mtime 相同时按文件名倒序，保证结果稳定）
    files.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| b.0.file_name().cmp(&a.0.file_name()))
    });

    let keep = keep as usize;
    if files.len() <= keep {
        return Ok(0);
    }
    let mut removed = 0usize;
    for (p, _) in files.into_iter().skip(keep) {
        if std::fs::remove_file(&p).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/* ============================ 命令 ============================ */

/// 列出全部定时备份任务
#[tauri::command]
pub fn list_schedules() -> Vec<BackupTask> {
    load_tasks()
}

/// 新增或更新任务（id 为空时生成 sch-YYYYMMDDHHMMSS）
#[tauri::command]
pub fn save_schedule(task: BackupTask) -> Result<Vec<BackupTask>, String> {
    let mut t = task;
    t.name = t.name.trim().to_string();
    if t.name.is_empty() {
        return Err("任务名称不能为空".into());
    }
    if !matches!(t.period.as_str(), "daily" | "weekly" | "monthly") {
        return Err("周期只能是每日 / 每周 / 每月".into());
    }
    // 能否解析出下次执行时间，即可判定 time + day 是否合法
    let now = Local::now().timestamp();
    if next_run_ts(now, &t.period, &t.time, t.day) == 0 {
        return Err("时间格式非法（应为 HH:MM），或星期/日期超出范围".into());
    }
    validate_keep(t.keep)?;

    let _g = lock_cfg();
    let mut list = read_tasks();

    if t.id.trim().is_empty() {
        let stamp = Local::now().format("%Y%m%d%H%M%S").to_string();
        let mut id = format!("sch-{}", stamp);
        let mut n = 1;
        while list.iter().any(|x| x.id == id) {
            n += 1;
            id = format!("sch-{}-{}", stamp, n);
        }
        t.id = id;
        // 首次保存：以当前时间为基准，避免刚建任务就立刻执行
        if t.last_run <= 0 {
            t.last_run = now;
        }
        list.push(t);
    } else {
        let Some(pos) = list.iter().position(|x| x.id == t.id) else {
            return Err("任务不存在（可能已被删除）".into());
        };
        // 前端可能没带执行记录，保留原值
        if t.last_run <= 0 {
            t.last_run = list[pos].last_run;
        }
        if t.last_result.is_empty() {
            t.last_result = list[pos].last_result.clone();
        }
        list[pos] = t;
    }

    write_tasks(&list)?;
    Ok(list)
}

/// 删除任务
#[tauri::command]
pub fn delete_schedule(id: String) -> Result<Vec<BackupTask>, String> {
    let _g = lock_cfg();
    let mut list = read_tasks();
    let before = list.len();
    list.retain(|t| t.id != id);
    if list.len() == before {
        return Err("任务不存在".into());
    }
    write_tasks(&list)?;
    Ok(list)
}

/// 启用 / 停用任务
#[tauri::command]
pub fn toggle_schedule(id: String, enabled: bool) -> Result<Vec<BackupTask>, String> {
    let _g = lock_cfg();
    let mut list = read_tasks();
    let Some(t) = list.iter_mut().find(|x| x.id == id) else {
        return Err("任务不存在".into());
    };
    t.enabled = enabled;
    write_tasks(&list)?;
    Ok(list)
}

/// 立即执行一次备份（复用 backup::create_backup）
#[tauri::command]
pub async fn run_schedule_now(id: String) -> Result<serde_json::Value, String> {
    let (name, keep) = {
        let _g = lock_cfg();
        let list = read_tasks();
        let t = list
            .iter()
            .find(|x| x.id == id)
            .ok_or_else(|| "任务不存在".to_string())?;
        (t.name.clone(), t.keep)
    };
    let dirs = load_dirs();
    let note = format!("定时任务「{}」手动执行", name);
    let res = backup::create_backup(note).await;
    let now = Local::now().timestamp();

    match res {
        Ok(v) => {
            let file = v
                .get("file")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            let fname = Path::new(&file)
                .file_name()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_else(|| file.clone());
            update_task_result(&id, now, &format!("成功：{}", fname))?;
            let removed = cleanup_in(&dirs.backup, keep).unwrap_or(0);
            op_log(
                &dirs,
                "schedule",
                "run_now",
                &format!(
                    "任务「{}」手动执行成功 -> {}{}",
                    name,
                    file,
                    if removed > 0 {
                        format!("（清理 {} 份旧备份）", removed)
                    } else {
                        String::new()
                    }
                ),
            );
            Ok(serde_json::json!({ "file": file, "time": now }))
        }
        Err(e) => {
            let short = short_text(&e, 120);
            let _ = update_task_result(&id, now, &format!("失败：{}", short));
            op_log(
                &dirs,
                "schedule",
                "run_now",
                &format!("任务「{}」手动执行失败: {}", name, e),
            );
            Err(e)
        }
    }
}

/// 清理备份目录下的旧备份，保留最新 keep 份，返回删除数量
#[tauri::command]
pub fn cleanup_old_backups(keep: u32) -> Result<usize, String> {
    if keep == 0 {
        return Err("至少保留 1 份".into());
    }
    if keep > 200 {
        return Err("保留份数过大".into());
    }
    let dirs = load_dirs();
    let removed = cleanup_in(&dirs.backup, keep)?;
    op_log(
        &dirs,
        "schedule",
        "cleanup_old_backups",
        &format!("保留最新 {} 份，删除 {} 份旧备份", keep, removed),
    );
    Ok(removed)
}

/// 调度状态：线程是否在跑 + 各启用任务的下次执行时间
#[tauri::command]
pub fn get_schedule_status() -> serde_json::Value {
    let now = Local::now().timestamp();
    let tasks = load_tasks();
    let mut next: Vec<serde_json::Value> = tasks
        .iter()
        .filter(|t| t.enabled)
        .filter_map(|t| {
            let base = if t.last_run > 0 { t.last_run } else { now };
            let ts = next_run_ts(base, &t.period, &t.time, t.day);
            if ts <= 0 {
                return None;
            }
            Some(serde_json::json!({
                "id": t.id,
                "name": t.name,
                "next_ts": ts,
                "next_text": format_ts(ts),
            }))
        })
        .collect();
    next.sort_by_key(|v| v.get("next_ts").and_then(|x| x.as_i64()).unwrap_or(0));
    serde_json::json!({
        "running": RUNNING.load(Ordering::Relaxed),
        "next": next,
    })
}

/* ============================ 后台调度线程 ============================ */

static STARTED: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);

fn format_ts(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn short_text(s: &str, max: usize) -> String {
    let t = s.replace(['\r', '\n'], " ");
    if t.chars().count() <= max {
        t
    } else {
        let cut: String = t.chars().take(max).collect();
        format!("{}…", cut)
    }
}

/// 执行一次备份（同步阻塞，内含 panic 兜底）
fn run_task_once(task: &BackupTask) -> Result<String, String> {
    let note = format!("定时任务「{}」自动备份", task.name);
    let joined = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tauri::async_runtime::block_on(backup::create_backup(note))
    }));
    match joined {
        Ok(Ok(v)) => Ok(v
            .get("file")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("执行备份时发生内部错误".into()),
    }
}

/// 单轮调度：找出所有到点的任务并执行
fn scheduler_tick() {
    let now = Local::now().timestamp();
    let tasks = load_tasks();
    for t in tasks {
        if !should_fire(now, &t) {
            continue;
        }
        // 单个任务出任何问题都不影响其它任务与线程本身
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let dirs = load_dirs();
            let fired_at = Local::now().timestamp();
            match run_task_once(&t) {
                Ok(file) => {
                    let fname = Path::new(&file)
                        .file_name()
                        .map(|x| x.to_string_lossy().into_owned())
                        .unwrap_or_else(|| file.clone());
                    let _ = update_task_result(&t.id, fired_at, &format!("成功：{}", fname));
                    let removed = cleanup_in(&dirs.backup, t.keep).unwrap_or(0);
                    op_log(
                        &dirs,
                        "schedule",
                        "auto_backup",
                        &format!(
                            "任务「{}」自动备份成功 -> {}{}",
                            t.name,
                            file,
                            if removed > 0 {
                                format!("（清理 {} 份旧备份）", removed)
                            } else {
                                String::new()
                            }
                        ),
                    );
                }
                Err(e) => {
                    let _ = update_task_result(
                        &t.id,
                        fired_at,
                        &format!("失败：{}", short_text(&e, 120)),
                    );
                    op_log(
                        &dirs,
                        "schedule",
                        "auto_backup",
                        &format!("任务「{}」自动备份失败: {}", t.name, e),
                    );
                }
            }
        }));
    }
}

/// 启动后台调度线程。父代理需要在 lib.rs 的 setup 中调用一次。
pub fn start_scheduler() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    RUNNING.store(true, Ordering::SeqCst);
    std::thread::spawn(|| {
        loop {
            // 整轮兜底：任何 panic 都不能让线程死掉
            let _ = std::panic::catch_unwind(scheduler_tick);
            std::thread::sleep(Duration::from_secs(TICK_SECS));
        }
    });
}

/* ============================ 测试 ============================ */

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone};

    /// 本地时间 y-m-d h:mi 的 Unix 秒（固定值，不依赖 now()）
    fn dt(y: i32, m: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, m, d, h, mi, 0)
            .unwrap()
            .timestamp()
    }

    fn local(y: i32, m: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    #[test]
    fn daily_next_is_today_when_future() {
        // 2026-10-18（周日）10:00，23:00 还没到 → 同一天
        let now = dt(2026, 10, 18, 10, 0);
        let next = next_run_ts(now, "daily", "23:00", 1);
        assert_eq!(next, dt(2026, 10, 18, 23, 0));
        let d = Local.timestamp_opt(next, 0).unwrap().date_naive();
        assert_eq!(d, Local.timestamp_opt(now, 0).unwrap().date_naive());
    }

    #[test]
    fn daily_next_is_tomorrow_when_past() {
        // 10:00 时 09:00 已过 → 次日
        let now = dt(2026, 10, 18, 10, 0);
        let next = next_run_ts(now, "daily", "09:00", 1);
        assert_eq!(next, dt(2026, 10, 19, 9, 0));
    }

    #[test]
    fn weekly_same_weekday_rolls_to_next_week() {
        // 2026-10-19 是周一，10:00 时 09:00 已过 → 下周一
        let now = dt(2026, 10, 19, 10, 0);
        let next = next_run_ts(now, "weekly", "09:00", 1);
        assert_eq!(next, dt(2026, 10, 26, 9, 0));
    }

    #[test]
    fn weekly_picks_target_weekday() {
        // 周日 10:00，目标周三（3）→ 本周三 09:00
        let now = dt(2026, 10, 18, 10, 0);
        let next = next_run_ts(now, "weekly", "09:00", 3);
        assert_eq!(next, dt(2026, 10, 21, 9, 0));
        let wd = Local
            .timestamp_opt(next, 0)
            .unwrap()
            .weekday()
            .num_days_from_monday()
            + 1;
        assert_eq!(wd, 3);

        // 目标周日（7）：本周日 09:00 已过 → 下周日
        let next_sun = next_run_ts(now, "weekly", "09:00", 7);
        assert_eq!(next_sun, dt(2026, 10, 25, 9, 0));
    }

    #[test]
    fn monthly_rolls_to_next_month() {
        // 2026-10-20 10:00，每月 5 日 03:00 → 11 月 5 日
        let now = dt(2026, 10, 20, 10, 0);
        let next = next_run_ts(now, "monthly", "03:00", 5);
        assert_eq!(next, dt(2026, 11, 5, 3, 0));
        let d = Local.timestamp_opt(next, 0).unwrap();
        assert_eq!(d.month(), 11);
        assert_eq!(d.day(), 5);

        // 跨年：12 月 20 日 → 次年 1 月 5 日
        let now2 = dt(2026, 12, 20, 10, 0);
        assert_eq!(next_run_ts(now2, "monthly", "03:00", 5), dt(2027, 1, 5, 3, 0));

        // 闰年 2 月：2028-02-10 → 2028-02-28
        let now3 = dt(2028, 2, 10, 10, 0);
        assert_eq!(
            next_run_ts(now3, "monthly", "03:00", 28),
            dt(2028, 2, 28, 3, 0)
        );
    }

    #[test]
    fn invalid_time_returns_zero() {
        let now = dt(2026, 10, 18, 10, 0);
        for bad in ["25:00", "abc", "", "12:60", "12:5", "1:2:3", "-1:00"] {
            assert_eq!(next_run_ts(now, "daily", bad, 1), 0, "非法时间 {:?}", bad);
        }
    }

    #[test]
    fn invalid_period_returns_zero() {
        let now = dt(2026, 10, 18, 10, 0);
        assert_eq!(next_run_ts(now, "yearly", "03:00", 1), 0);
        assert_eq!(next_run_ts(now, "", "03:00", 1), 0);
        // day 越界
        assert_eq!(next_run_ts(now, "weekly", "03:00", 0), 0);
        assert_eq!(next_run_ts(now, "weekly", "03:00", 8), 0);
        assert_eq!(next_run_ts(now, "monthly", "03:00", 29), 0);
    }

    #[test]
    fn should_fire_respects_elapsed() {
        let base = dt(2026, 10, 18, 10, 0); // 基准 10:00，任务时间 09:00
        // 次日 10:00：已经过点 → 触发
        assert!(should_fire_at(
            dt(2026, 10, 19, 10, 0),
            base,
            "daily",
            "09:00",
            1
        ));
        // 当日 11:00：下一次是次日 09:00，尚未到 → 不触发
        assert!(!should_fire_at(
            dt(2026, 10, 18, 11, 0),
            base,
            "daily",
            "09:00",
            1
        ));
        // 恰好到点 → 触发
        assert!(should_fire_at(
            dt(2026, 10, 19, 9, 0),
            base,
            "daily",
            "09:00",
            1
        ));
        // 配置非法 → 不触发
        assert!(!should_fire_at(dt(2026, 10, 19, 9, 0), base, "daily", "xx", 1));
    }

    #[test]
    fn keep_validation() {
        assert!(validate_keep(0).is_err());
        assert!(validate_keep(1).is_ok());
        assert!(validate_keep(50).is_ok());
        assert!(validate_keep(51).is_err());
    }

    #[test]
    fn task_deserializes_with_defaults() {
        // 缺省字段按 serde 默认值补齐
        let t: BackupTask = serde_json::from_str(r#"{"id":"a","name":"n","period":"daily","time":"03:00"}"#)
            .expect("应可反序列化");
        assert_eq!(t.day, 1);
        assert!(t.enabled);
        assert_eq!(t.keep, 5);
        assert_eq!(t.last_run, 0);
        assert!(t.apps.is_empty());
    }

    #[test]
    fn local_helper_sanity() {
        // 固定日期的星期几自检（2026-10-18 为周日）
        assert_eq!(local(2026, 10, 18, 10, 0).weekday().number_from_monday(), 7);
    }
}
