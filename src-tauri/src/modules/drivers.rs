//! 驱动清单：pnputil /enum-drivers 解析与 CSV 导出
//!
//! 解析兼容性：
//! - 中文系统输出「发布名称 / 原始名称 / 提供程序名称 / 驱动程序版本」，英文系统
//!   输出「Published Name / Original Name / Provider Name / Driver Version」，
//!   按 `标签: 值` 的标签尾部匹配两套前缀，与缩进、冒号旁空格无关；
//! - pnputil 重定向输出默认走 OEM 代码页（中文系统为 GBK），这里用
//!   `cmd /c "chcp 65001 >nul & pnputil /enum-drivers"` 强制 UTF-8 后按行解析
//!   （chcp 65001 下部分系统会切换为英文标签，两套前缀均已覆盖）；
//! - 首条记录前有「Microsoft PnP 工具/Utility」标题行（可能与首字段粘连在同一行），
//!   按标签尾部匹配可正确剥离；
//! - 「驱动程序版本」行格式为 `MM/DD/YYYY x.y.z.w`，拆分为 date 与 version 两列。

use serde::Serialize;
use serde_json::json;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 单条驱动记录
#[derive(Serialize)]
pub struct DriverItem {
    pub device_name: String,
    pub version: String,
    pub provider: String,
    pub date: String,
    pub inf_name: String,
}

// 字段槽位：0=inf_name 1=device_name 2=provider 3=date 4=version
const FIELDS: &[(&[&str], usize)] = &[
    (&["发布名称", "Published Name"], 0),
    (&["原始名称", "Original Name"], 1),
    (&["提供程序名称", "Provider Name"], 2),
    (&["驱动程序版本", "Driver Version"], 3),
];

/// 执行 pnputil（强制 UTF-8 输出），失败返回 None
fn pnputil_output() -> Option<String> {
    let out = std::process::Command::new("cmd")
        .args(["/c", "chcp 65001 >nul & pnputil /enum-drivers"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 解析 pnputil 输出为驱动条目列表（中英文标签双兼容）
fn parse_drivers(text: &str) -> Vec<DriverItem> {
    fn flush(out: &mut Vec<DriverItem>, cur: &mut [String; 5]) {
        if !cur[0].trim().is_empty() {
            out.push(DriverItem {
                inf_name: cur[0].trim().to_string(),
                device_name: if cur[1].trim().is_empty() {
                    "未知".into()
                } else {
                    cur[1].trim().to_string()
                },
                provider: cur[2].trim().to_string(),
                date: cur[3].trim().to_string(),
                version: cur[4].trim().to_string(),
            });
        }
        for f in cur.iter_mut() {
            f.clear();
        }
    }

    let mut out: Vec<DriverItem> = Vec::new();
    let mut cur: [String; 5] = std::array::from_fn(|_| String::new());

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush(&mut out, &mut cur);
            continue;
        }
        // 在第一个半/全角冒号处拆分「标签: 值」
        let (label, value) = match line.split_once(':') {
            Some((l, v)) => (l, v.trim()),
            None => continue,
        };
        let label = label.trim_end_matches('：').trim();
        for (names, slot) in FIELDS {
            if names.iter().any(|n| label.ends_with(n)) {
                if *slot == 3 {
                    // 「驱动程序版本: 07/18/1968 10.1.37.7」→ date + version
                    let mut it = value.split_whitespace();
                    let first = it.next().unwrap_or("");
                    let rest = it.collect::<Vec<_>>().join(" ");
                    if first.contains('/') {
                        cur[3] = first.to_string();
                        cur[4] = rest;
                    } else {
                        // 旧版 pnputil 无日期，仅版本号
                        cur[4] = value.to_string();
                    }
                } else {
                    cur[*slot] = value.to_string();
                }
            }
        }
    }
    flush(&mut out, &mut cur);
    out
}

/// 枚举第三方驱动包清单（pnputil /enum-drivers）
#[tauri::command]
pub async fn driver_list() -> Result<Vec<DriverItem>, String> {
    let text = pnputil_output().ok_or("无法执行 pnputil（请检查系统环境）")?;
    let list = parse_drivers(&text);
    if list.is_empty() {
        return Err("未解析到任何驱动条目（pnputil 输出格式可能不兼容）".into());
    }
    Ok(list)
}

/// CSV 字段转义：含逗号/引号/换行时加引号，内部引号翻倍
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// 导出驱动清单为 CSV（UTF-8-BOM，Excel 友好）到 data\备份导出\drivers-YYYYMMDD.csv
#[tauri::command]
pub async fn export_driver_list() -> Result<serde_json::Value, String> {
    let text = pnputil_output().ok_or("无法执行 pnputil（请检查系统环境）")?;
    let list = parse_drivers(&text);
    if list.is_empty() {
        return Err("未解析到任何驱动条目（pnputil 输出格式可能不兼容）".into());
    }

    let dirs = crate::dirs::load_dirs();
    let file = PathBuf::from(&dirs.backup).join(format!(
        "drivers-{}.csv",
        chrono::Local::now().format("%Y%m%d")
    ));
    fs::create_dir_all(&dirs.backup).map_err(|e| format!("创建备份导出目录失败: {}", e))?;

    let mut csv = String::from("\u{FEFF}设备名称,驱动版本,提供程序,日期,INF名称\r\n");
    for d in &list {
        csv.push_str(&format!(
            "{},{},{},{},{}\r\n",
            csv_field(&d.device_name),
            csv_field(&d.version),
            csv_field(&d.provider),
            csv_field(&d.date),
            csv_field(&d.inf_name),
        ));
    }
    fs::write(&file, csv.as_bytes()).map_err(|e| format!("写入 CSV 失败: {}", e))?;

    let count = list.len();
    crate::logs::op_log(
        &dirs,
        "drivers",
        "export_driver_list",
        &format!("导出 {} 条驱动清单到 {}", count, file.display()),
    );
    Ok(json!({
        "file": file.to_string_lossy(),
        "count": count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chinese() {
        let text = "Microsoft PnP \u{5de5}\u{5177}\u{53d1}\u{5e03}\u{540d}\u{79f0}:     oem10.inf\r\n\
                    \u{539f}\u{59cb}\u{540d}\u{79f0}:      alderlake.inf\r\n\
                    \u{63d0}\u{4f9b}\u{7a0b}\u{5e8f}\u{540d}\u{79f0}:      INTEL\r\n\
                    \u{7c7b}\u{540d}:         \u{7cfb}\u{7edf}\u{8bbe}\u{5907}\r\n\
                    \u{9a71}\u{52a8}\u{7a0b}\u{5e8f}\u{7248}\u{672c}:     07/18/1968 10.1.37.7\r\n\
                    \u{7b7e}\u{540d}\u{8005}\u{59d3}\u{540d}:        Microsoft\r\n\r\n";
        let list = parse_drivers(text);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].inf_name, "oem10.inf");
        assert_eq!(list[0].device_name, "alderlake.inf");
        assert_eq!(list[0].provider, "INTEL");
        assert_eq!(list[0].date, "07/18/1968");
        assert_eq!(list[0].version, "10.1.37.7");
    }

    #[test]
    fn parse_english() {
        let text = "Published Name:     oem29.inf\r\nOriginal Name:      netlvss.inf\r\nProvider Name:      Realtek\r\nClass Name:         Net\r\nDriver Version:     03/01/2024 11.2.5.1\r\nSigner Name:        Microsoft\r\n\r\n";
        let list = parse_drivers(text);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].inf_name, "oem29.inf");
        assert_eq!(list[0].device_name, "netlvss.inf");
        assert_eq!(list[0].provider, "Realtek");
        assert_eq!(list[0].date, "03/01/2024");
        assert_eq!(list[0].version, "11.2.5.1");
    }

    #[test]
    fn parse_multiple_records() {
        // 两条记录以空行分隔，末条无尾随空行也应被 flush
        let text = "Published Name:     oem1.inf\r\nOriginal Name:      a.inf\r\nProvider Name:      A Corp\r\nDriver Version:     01/02/2023 1.0.0.0\r\n\r\nPublished Name:     oem2.inf\r\nOriginal Name:      b.inf\r\nProvider Name:      B Corp\r\nDriver Version:     04/05/2026 2.0.0.0\r\n";
        let list = parse_drivers(text);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].inf_name, "oem1.inf");
        assert_eq!(list[1].inf_name, "oem2.inf");
        assert_eq!(list[1].version, "2.0.0.0");
    }

    #[test]
    fn parse_missing_original_name_defaults_unknown() {
        // 缺「原始名称」字段 → device_name 兜底 "未知"
        let text = "Published Name:     oem7.inf\r\nProvider Name:      Intel\r\nDriver Version:     06/07/2025 9.9.9.9\r\n";
        let list = parse_drivers(text);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].device_name, "未知");
        assert_eq!(list[0].inf_name, "oem7.inf");
    }

    #[test]
    fn parse_old_pnputil_version_without_date() {
        // 旧版 pnputil：驱动程序版本行仅有版本号、无日期
        let text = "发布名称:     oem5.inf\r\n原始名称:     legacy.inf\r\n提供程序名称: Foo\r\n驱动程序版本:     12.34.56.78\r\n";
        let list = parse_drivers(text);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].date, "");
        assert_eq!(list[0].version, "12.34.56.78");
    }

    #[test]
    fn parse_empty_and_noise_lines() {
        // 无标签的标题行与空输入不产生记录
        assert!(parse_drivers("").is_empty());
        assert!(parse_drivers("Microsoft PnP Utility\r\n\r\n").is_empty());
    }

    #[test]
    fn csv_field_escaping() {
        // 普通值不加引号
        assert_eq!(csv_field("plain"), "plain");
        // 含逗号/引号/换行加引号，内部引号翻倍
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_field("a\nb"), "\"a\nb\"");
    }
}
