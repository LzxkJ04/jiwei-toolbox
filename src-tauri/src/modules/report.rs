//! 系统信息导出：把硬件、系统、授权、已装软件汇总成一份可读报告
//!
//! 导出两份文件：
//! - `系统报告-YYYYMMDD-HHMMSS.txt`：给人看的纯文本
//! - `系统报告-YYYYMMDD-HHMMSS.md`：Markdown，方便贴到工单/聊天
//!
//! 数据全部复用已有命令（hw_info / license_status / list_installed），
//! 不重复采集，避免两处结果不一致。

use serde::Serialize;
use std::fmt::Write as _;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct ReportResult {
    pub txt_file: String,
    pub md_file: String,
    /// 软件条数
    pub app_count: usize,
    pub size: u64,
}

fn fmt_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if b == 0 {
        return "0 B".to_string();
    }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// 从 JSON 里安全取字符串
fn s(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// 取数字并格式化
fn n(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

/// 生成报告正文。返回 (txt, md)
fn build_report(
    hw: &serde_json::Value,
    lic: &serde_json::Value,
    apps: &[serde_json::Value],
    now: &str,
    app_version: &str,
) -> (String, String) {
    let mut t = String::new();
    let mut m = String::new();

    let cpu = hw.get("cpu").cloned().unwrap_or(serde_json::Value::Null);
    let os = hw.get("os").cloned().unwrap_or(serde_json::Value::Null);
    let bios = hw.get("bios").cloned().unwrap_or(serde_json::Value::Null);
    let mb = hw.get("motherboard").cloned().unwrap_or(serde_json::Value::Null);

    // ---------- 页眉 ----------
    let _ = writeln!(t, "极维工具箱 - 整机系统报告");
    let _ = writeln!(t, "{}", "=".repeat(48));
    let _ = writeln!(t, "生成时间：{}", now);
    let _ = writeln!(t, "程序版本：极维工具箱 v{}", app_version);
    let os_caption = {
        let c = s(&os, "caption");
        if c.is_empty() { s(&lic, "osEdition") } else { c }
    };
    let os_version = {
        let v = s(&os, "version");
        if v.is_empty() { s(&lic, "osVersion") } else { v }
    };
    let os_build = s(&os, "build");
    let os_arch = s(&os, "arch");
    let _ = writeln!(
        t,
        "操作系统：{} {}{}",
        if os_caption.is_empty() { "未知" } else { &os_caption },
        os_version,
        if os_build.is_empty() {
            String::new()
        } else {
            format!(" (Build {})", os_build)
        }
    );
    if !os_arch.is_empty() {
        let _ = writeln!(t, "系统架构：{}", os_arch);
    }
    let install_date = s(&lic, "installDate");
    if !install_date.is_empty() {
        let _ = writeln!(t, "系统安装：{}", install_date);
    }
    let _ = writeln!(t);

    let _ = writeln!(m, "# 极维工具箱｜整机系统报告\n");
    let _ = writeln!(m, "- 生成时间：{}", now);
    let _ = writeln!(m, "- 程序版本：极维工具箱 v{}", app_version);
    let _ = writeln!(
        m,
        "- 操作系统：{} {} Build {}",
        if os_caption.is_empty() { "未知" } else { &os_caption },
        os_version,
        os_build
    );
    if !install_date.is_empty() {
        let _ = writeln!(m, "- 系统安装：{}", install_date);
    }
    let _ = writeln!(m);

    // ---------- 一、整机摘要 ----------
    let mem_total = n(hw, "memory_total");
    let mut disk_total = 0.0;
    if let Some(arr) = hw.get("disks").and_then(|x| x.as_array()) {
        for d in arr {
            disk_total += n(d, "size");
        }
    }
    let _ = writeln!(t, "【一、整机摘要】");
    let _ = writeln!(t, "  CPU        : {}", non_empty_or(&s(&cpu, "name"), "未知"));
    let _ = writeln!(
        t,
        "  核心/线程  : {} 核 {} 线程",
        n(&cpu, "cores") as u64,
        n(&cpu, "threads") as u64
    );
    let _ = writeln!(t, "  内存总量   : {}", fmt_bytes(mem_total as u64));
    if let Some(gpus) = hw.get("gpus").and_then(|x| x.as_array()) {
        for g in gpus.iter().filter(|g| !g.get("virtual").and_then(|v| v.as_bool()).unwrap_or(false)) {
            let vram = n(g, "vram") as u64;
            let _ = writeln!(
                t,
                "  显卡       : {}{}",
                s(g, "name"),
                if vram > 0 {
                    format!("（{}）", fmt_bytes(vram * 1024 * 1024))
                } else {
                    String::new()
                }
            );
        }
    }
    if let Some(disks) = hw.get("disks").and_then(|x| x.as_array()) {
        for d in disks {
            let _ = writeln!(
                t,
                "  磁盘       : {} · {} · {} · {}",
                non_empty_or(&s(d, "model"), "未知磁盘"),
                fmt_bytes(n(d, "size") as u64),
                s(d, "bus"),
                s(d, "health")
            );
        }
    }
    let _ = writeln!(t, "  磁盘合计   : {}", fmt_bytes(disk_total as u64));
    let _ = writeln!(t, "  授权状态   : {}", non_empty_or(&s(&lic, "statusText"), "未知"));
    let _ = writeln!(t);

    let _ = writeln!(m, "## 一、整机摘要\n");
    let _ = writeln!(m, "| 项目 | 值 |");
    let _ = writeln!(m, "| --- | --- |");
    let _ = writeln!(m, "| CPU | {} |", non_empty_or(&s(&cpu, "name"), "未知"));
    let _ = writeln!(
        m,
        "| 核心/线程 | {} 核 {} 线程 |",
        n(&cpu, "cores") as u64,
        n(&cpu, "threads") as u64
    );
    let _ = writeln!(m, "| 内存总量 | {} |", fmt_bytes(mem_total as u64));
    let _ = writeln!(m, "| 磁盘合计 | {} |", fmt_bytes(disk_total as u64));
    let _ = writeln!(m, "| 授权状态 | {} |", non_empty_or(&s(&lic, "statusText"), "未知"));
    let _ = writeln!(m);

    // ---------- 二、处理器 ----------
    let _ = writeln!(t, "【二、处理器】");
    for (k, key) in [
        ("型号", "name"),
        ("厂商", "vendor"),
        ("架构", "arch"),
        ("插槽", "socket"),
        ("核心数", "cores"),
        ("线程数", "threads"),
        ("基准频率", "mhz"),
        ("当前频率", "current_mhz"),
        ("最高频率", "max_mhz"),
        ("二级缓存", "l2_kb"),
        ("三级缓存", "l3_kb"),
        ("硬件虚拟化", "virtualization"),
    ] {
        let val = s(&cpu, key);
        let num = n(&cpu, key);
        let text = if !val.is_empty() {
            val
        } else if num > 0.0 {
            if key.ends_with("_kb") {
                format!("{} KB", num as u64)
            } else if key.contains("mhz") {
                format!("{} MHz", num as u64)
            } else {
                format!("{}", num as u64)
            }
        } else {
            continue;
        };
        let _ = writeln!(t, "  {:<10}: {}", k, text);
    }
    let _ = writeln!(t);

    // ---------- 三、内存 ----------
    let _ = writeln!(t, "【三、内存】");
    let _ = writeln!(t, "  总容量     : {}", fmt_bytes(mem_total as u64));
    if let Some(mods) = hw.get("memoryModules").and_then(|x| x.as_array()) {
        for (i, mm) in mods.iter().enumerate() {
            let _ = writeln!(
                t,
                "  条 {}       : {} {} · {} GB · {} MHz · {}",
                i + 1,
                non_empty_or(&s(mm, "type"), "未知"),
                non_empty_or(&s(mm, "manufacturer"), "未知厂商"),
                n(mm, "capacity_gb"),
                n(mm, "speed") as u64,
                non_empty_or(&s(mm, "slot"), "未报告插槽")
            );
        }
    }
    let _ = writeln!(t);

    // ---------- 四、显卡 ----------
    let _ = writeln!(t, "【四、显卡】");
    if let Some(gpus) = hw.get("gpus").and_then(|x| x.as_array()) {
        for g in gpus {
            let virtual_tag = if g.get("virtual").and_then(|v| v.as_bool()).unwrap_or(false) {
                "（虚拟显示器）"
            } else {
                ""
            };
            let _ = writeln!(t, "  {}", s(g, "name"));
            if !virtual_tag.is_empty() {
                let _ = writeln!(t, "    类型     : {}", virtual_tag);
            }
            let vram = n(g, "vram") as u64;
            if vram > 0 {
                let _ = writeln!(t, "    显存     : {}", fmt_bytes(vram * 1024 * 1024));
            }
            let drv = s(g, "driver");
            if !drv.is_empty() {
                let _ = writeln!(t, "    驱动     : {}", drv);
            }
        }
    }
    let _ = writeln!(t);

    // ---------- 五、磁盘 ----------
    let _ = writeln!(t, "【五、磁盘】");
    if let Some(disks) = hw.get("disks").and_then(|x| x.as_array()) {
        for d in disks {
            let _ = writeln!(t, "  {}", non_empty_or(&s(d, "model"), "未知磁盘"));
            let _ = writeln!(t, "    容量     : {}", fmt_bytes(n(d, "size") as u64));
            let bus = s(d, "bus");
            if !bus.is_empty() {
                let _ = writeln!(t, "    接口     : {}", bus);
            }
            let media = s(d, "media");
            if !media.is_empty() {
                let _ = writeln!(t, "    介质     : {}", media);
            }
            let health = s(d, "health");
            if !health.is_empty() {
                let _ = writeln!(t, "    健康     : {}", health);
            }
            let serial = s(d, "serial");
            if !serial.is_empty() {
                let _ = writeln!(t, "    序列号   : {}", serial);
            }
        }
    }
    if let Some(vols) = hw.get("volumes").and_then(|x| x.as_array()) {
        for v in vols {
            let total = n(v, "total") as u64;
            let free = n(v, "free") as u64;
            let used_pct = if total > 0 {
                ((total - free) as f64 / total as f64 * 100.0).round()
            } else {
                0.0
            };
            let _ = writeln!(
                t,
                "  分区 {} : {} / {}（已用 {:.0}%）",
                s(v, "mount"),
                fmt_bytes(total - free),
                fmt_bytes(total),
                used_pct
            );
        }
    }
    let _ = writeln!(t);

    // ---------- 六、主板 ----------
    let _ = writeln!(t, "【六、主板与 BIOS】");
    let _ = writeln!(t, "  主板型号   : {}", non_empty_or(&s(hw, "board"), "未知"));
    let serial = s(&mb, "serial");
    if !serial.is_empty() {
        let _ = writeln!(t, "  主板序列号 : {}", serial);
    }
    let _ = writeln!(
        t,
        "  BIOS       : {} {}",
        non_empty_or(&s(&bios, "vendor"), "未知厂商"),
        s(&bios, "version")
    );
    let bd = s(&bios, "date");
    if !bd.is_empty() {
        let _ = writeln!(t, "  BIOS 日期  : {}", bd);
    }
    let _ = writeln!(t);

    // ---------- 七、已安装软件 ----------
    let _ = writeln!(t, "【七、已安装软件（{} 个）】", apps.len());
    let mut sorted: Vec<&serde_json::Value> = apps.iter().collect();
    sorted.sort_by(|a, b| {
        s(a, "name")
            .to_lowercase()
            .cmp(&s(b, "name").to_lowercase())
    });
    for (i, a) in sorted.iter().enumerate() {
        let ver = s(a, "version");
        let pub_name = s(a, "publisher");
        let _ = writeln!(
            t,
            "  {:>3}. {}{}{}",
            i + 1,
            s(a, "name"),
            if ver.is_empty() {
                String::new()
            } else {
                format!("  v{}", ver)
            },
            if pub_name.is_empty() {
                String::new()
            } else {
                format!("  [{}]", pub_name)
            }
        );
    }
    let _ = writeln!(t);
    let _ = writeln!(t, "报告结束。数据由本机注册表与 WMI 读取，仅供参考。");

    let _ = writeln!(m, "## 二、已安装软件（{} 个）\n", apps.len());
    let _ = writeln!(m, "| 软件 | 版本 | 发行商 |");
    let _ = writeln!(m, "| --- | --- | --- |");
    for a in sorted.iter().take(300) {
        let _ = writeln!(
            m,
            "| {} | {} | {} |",
            s(a, "name"),
            s(a, "version"),
            s(a, "publisher")
        );
    }
    if apps.len() > 300 {
        let _ = writeln!(m, "\n> 仅列出前 300 个，完整清单见 txt 报告。");
    }
    let _ = writeln!(m, "\n---\n数据由本机注册表与 WMI 读取，仅供参考。");

    (t, m)
}

fn non_empty_or(v: &str, fallback: &str) -> String {
    if v.trim().is_empty() {
        fallback.to_string()
    } else {
        v.to_string()
    }
}

/// 导出整机系统报告到指定目录（为空则用备份导出目录）。
#[tauri::command]
pub async fn export_system_report(target: Option<String>) -> Result<ReportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dirs = crate::dirs::load_dirs();
        let dir = match target.as_deref() {
            Some(t) if !t.trim().is_empty() => PathBuf::from(t),
            _ => PathBuf::from(&dirs.backup),
        };
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败：{e}"))?;

        let hw = crate::modules::hardware::hw_info_sync();
        let lic = serde_json::to_value(crate::modules::license::license_status_sync())
            .unwrap_or(serde_json::Value::Null);
        let apps: Vec<serde_json::Value> = crate::modules::uninstall::list_installed_inner()
            .into_iter()
            .map(|a| {
                serde_json::json!({
                    "name": a.name,
                    "version": a.version,
                    "publisher": a.publisher,
                })
            })
            .collect();

        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let version = env!("CARGO_PKG_VERSION");
        let (txt, md) = build_report(&hw, &lic, &apps, &now, version);

        let txt_file = dir.join(format!("系统报告-{stamp}.txt"));
        let md_file = dir.join(format!("系统报告-{stamp}.md"));
        std::fs::write(&txt_file, txt.as_bytes()).map_err(|e| format!("写入 txt 失败：{e}"))?;
        std::fs::write(&md_file, md.as_bytes()).map_err(|e| format!("写入 md 失败：{e}"))?;

        let size = std::fs::metadata(&txt_file).map(|m| m.len()).unwrap_or(0);
        crate::logs::op_log(
            &dirs,
            "report",
            "export_system",
            &format!("导出系统报告：{} 个软件，{} 字节", apps.len(), size),
        );

        Ok(ReportResult {
            txt_file: txt_file.to_string_lossy().to_string(),
            md_file: md_file.to_string_lossy().to_string(),
            app_count: apps.len(),
            size,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::{build_report, fmt_bytes};
    use serde_json::json;

    #[test]
    fn bytes_are_human_readable() {
        assert_eq!(fmt_bytes(0), "0 B");
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536), "1.5 KB");
        assert_eq!(fmt_bytes(1024 * 1024 * 2), "2.0 MB");
        assert_eq!(fmt_bytes(1024u64 * 1024 * 1024 * 16), "16.0 GB");
    }

    #[test]
    fn report_contains_core_sections() {
        let hw = json!({
            "cpu": { "name": "Intel Core i5-14600KF", "cores": 14, "threads": 20, "mhz": 3500 },
            "memory_total": 34359738368u64,
            "memoryModules": [
                { "manufacturer": "Samsung", "capacity_gb": 16.0, "speed": 3200, "slot": "DIMM1", "type": "DDR4" }
            ],
            "motherboard": { "manufacturer": "HUANANZHI", "product": "B760M-D4", "version": "V2.0", "serial": "SN1" },
            "board": "HUANANZHI B760M-D4 V2.0",
            "bios": { "vendor": "American Megatrends", "version": "5.27", "date": "2024-03-06" },
            "os": { "caption": "Microsoft Windows 10 Pro", "version": "22H2", "build": "19045", "arch": "64-bit" },
            "gpus": [
                { "name": "NVIDIA GeForce RTX 3060", "vram": 12288, "driver": "31.0.15.3699", "virtual": false },
                { "name": "OrayIddDriver", "vram": 0, "driver": "", "virtual": true }
            ],
            "disks": [ { "model": "WD PC SN810", "size": 512110190592u64, "bus": "NVMe", "media": "SSD", "health": "Healthy", "serial": "SN123" } ],
            "volumes": [ { "mount": "C:", "total": 500000000000u64, "free": 200000000000u64 } ]
        });
        let lic = json!({
            "osEdition": "Microsoft Windows 10 Pro",
            "osVersion": "22H2",
            "statusText": "已授权",
            "installDate": "2025-02-24"
        });
        let apps = vec![
            json!({ "name": "7-Zip", "version": "24.09", "publisher": "Igor Pavlov" }),
            json!({ "name": "Google Chrome", "version": "130.0", "publisher": "Google LLC" }),
        ];
        let (txt, md) = build_report(&hw, &lic, &apps, "2026-10-18 13:00:00", "0.1.0");

        assert!(txt.contains("整机系统报告"));
        assert!(txt.contains("Intel Core i5-14600KF"));
        assert!(txt.contains("14 核 20 线程"));
        assert!(txt.contains("32.0 GB"));
        assert!(txt.contains("12.0 GB"), "显存应按 MB→GB 换算");
        assert!(txt.contains("已授权"));
        assert!(txt.contains("已安装软件（2 个）"));
        // 虚拟显示器要标出来，不能混在真实显卡里
        assert!(txt.contains("虚拟显示器"));
        // 分区占用百分比要算对：300/500 = 60%
        assert!(txt.contains("已用 60%"));
        // Windows 10 是 19045，不能因为低于 22000 就漏掉
        assert!(txt.contains("Build 19045"));

        assert!(md.starts_with("# 极维工具箱"));
        assert!(md.contains("| 7-Zip | 24.09 | Igor Pavlov |"));
    }

    #[test]
    fn report_survives_empty_snapshot() {
        // WMI 全挂时不能 panic，要有兜底文案
        let (txt, _md) = build_report(
            &json!({}),
            &json!({}),
            &[],
            "2026-10-18 13:00:00",
            "0.1.0",
        );
        assert!(txt.contains("未知"));
        assert!(txt.contains("已安装软件（0 个）"));
    }

    /// 现场探针：真机导出一次，确认文件落盘且内容非空。
    /// `cargo test --lib live_export_report -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_export_report() {
        let dir = std::env::temp_dir().join("jiwei-report-probe");
        let res = tauri::async_runtime::block_on(super::export_system_report(Some(
            dir.to_string_lossy().to_string(),
        )))
        .expect("导出应该成功");
        println!(
            "apps={} size={} txt={}",
            res.app_count, res.size, res.txt_file
        );
        let text = std::fs::read_to_string(&res.txt_file).unwrap();
        println!("{}", text.lines().take(22).collect::<Vec<_>>().join("\n"));
        assert!(res.app_count > 0, "本机应有已安装软件");
        assert!(text.contains("整机系统报告"));
        assert!(text.contains("已安装软件（"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
