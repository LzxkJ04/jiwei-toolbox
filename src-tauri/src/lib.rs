//! 极维工具箱 - Tauri 2 后端入口
//!
//! 模块划分：
//! - dirs      目录定位
//! - software  软件清单与批量静默安装
//! - uninstall 已安装枚举与残留清理
//! - hardware  硬件信息 / CPU 烤机 / 磁盘测速
//! - systools  启动项 / 服务 / hosts / 运行库与 UWP（桩）
//! - backup    配置备份与恢复
//! - cache     垃圾清理与缓存统计
//! - logs      日志读取
//! - common    环境检测 / 设置 / 自启
//! - single_instance 单实例（互斥体 + 激活已有窗口）
//! - launcher   引导壳三模式（主程序 / --installer / --uninstaller）

mod launcher;
mod modules;
mod single_instance;

/// startup_backup 供 systools 内部使用，这里再提一层方便 `crate::startup_backup::`
pub(crate) use modules::startup_backup;

/// 兼容脚手架保留
#[allow(dead_code)]
#[tauri::command]
fn greet(name: &str) -> String {
    format!("你好, {}! 来自极维工具箱后端。", name)
}

use modules::{appicon, backup, cache, common, dirs, diskspace, drivers, hardware, inventory, license, logs, maintenance, netclient, software, sysmon, sysquick, systools, taskopts, templates, uninstall, updater, webview2};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{MenuBuilder, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewWindow,
};

/// "已隐藏到托盘"提示是否已发送（只提示一次）
static TRAY_HINT_EMITTED: AtomicBool = AtomicBool::new(false);

/// 窗口边界（物理像素），用于位置记忆
#[derive(serde::Serialize, serde::Deserialize)]
struct WinBounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    maximized: bool,
}

/// 显示、还原并聚焦主窗口
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// 真正退出：保存窗口边界后结束进程
fn do_exit(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        save_win_bounds(&win);
    }
    app.exit(0);
}

/// 前端确认后强制退出（先终止任务队列，不删除缓存）
#[tauri::command]
fn force_exit(app: tauri::AppHandle) {
    software::terminate_tasks(app.clone(), false);
    do_exit(&app);
}

/// 保存窗口边界到 settings.json（key: win_bounds）
fn save_win_bounds(window: &WebviewWindow) {
    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        return;
    };
    let bounds = WinBounds {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
        maximized: window.is_maximized().unwrap_or(false),
    };
    if let Ok(v) = serde_json::to_value(&bounds) {
        let _ = common::write_setting_raw("win_bounds", v);
    }
}

/// 启动时按 settings.json 记录还原窗口边界（无记录则保持 tauri.conf 初始值）
fn restore_win_bounds(window: &WebviewWindow) {
    let Some(v) = common::read_setting_raw("win_bounds") else {
        return;
    };
    let Ok(b) = serde_json::from_value::<WinBounds>(v) else {
        return;
    };
    if b.maximized {
        let _ = window.maximize();
    } else {
        // 忽略完全不在屏幕上的历史位置风险较小：用户可通过再次拖动纠正，
        // 这里仍信任记录（物理坐标与原 DPI 匹配）
        let _ = window.set_position(PhysicalPosition::new(b.x, b.y));
        let _ = window.set_size(PhysicalSize::new(b.width, b.height));
    }
}

/// 构建系统托盘：显示主窗口 / 批量安装 / --- / 任务控制与清理 / --- / 退出
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "tray-show", "显示主窗口", true, None::<&str>)?;
    let install = MenuItem::with_id(app, "tray-install", "批量安装", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let pause = MenuItem::with_id(app, "tray-pause", "暂停全部任务", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "tray-resume", "继续任务", true, None::<&str>)?;
    let clean = MenuItem::with_id(app, "tray-clean", "清理本地缓存", true, None::<&str>)?;
    let logs = MenuItem::with_id(app, "tray-logs", "打开日志中心", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "tray-quit", "退出极维工具箱", true, None::<&str>)?;
    let menu = MenuBuilder::new(app)
        .items(&[&show, &install, &sep, &pause, &resume, &clean, &logs, &sep2, &quit])
        .build()?;

    TrayIconBuilder::with_id("main-tray")
        .icon(tauri::include_image!("icons/32x32.png"))
        .tooltip("极维工具箱")
        .menu(&menu)
        // 左键单击直接显示主窗口，而不是弹出菜单（右键弹菜单）
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-show" => show_main_window(app),
            "tray-install" => {
                show_main_window(app);
                let _ = app.emit("navigate", serde_json::json!({ "page": "library" }));
            }
            "tray-pause" => software::pause_tasks(app.clone()),
            "tray-resume" => software::resume_tasks(app.clone()),
            "tray-clean" => {
                cache::clean_cache_inner();
                let _ = app.emit("tray-notify", "本地缓存已清理");
            }
            "tray-logs" => {
                show_main_window(app);
                let _ = app.emit("navigate", serde_json::json!({ "page": "logs" }));
            }
            "tray-quit" => {
                // 任务运行中：弹前端确认窗（退出会终止所有任务）
                let st = software::get_queue_state();
                if st.running {
                    show_main_window(app);
                    let _ = app.emit("exit-confirm-request", ());
                } else {
                    do_exit(app);
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    // 首次调用即解析并缓存启动模式（main / installer / uninstaller / repair / unattended）
    let mode = launcher::detect_launch_mode();

    // 无人值守部署：无 UI，读 unattend.json 静默安装后直接退出进程（不进入 Tauri 运行时）
    if mode == launcher::LaunchMode::Unattended {
        std::process::exit(launcher::run_unattended());
    }

    // 单实例：仅主程序模式启用；安装/卸载/修复模式允许多实例并存（便于测试与
    // 覆盖安装场景：从 {app} 里的壳再次安装到别处）
    if launcher::is_main_mode() {
        single_instance::acquire_or_exit();
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            // 目录（双路径模型）
            dirs::get_dirs,
            dirs::set_dirs,
            dirs::read_legacy_dirs,
            // 环境 / 设置 / 自启 / 收藏
            common::env_status,
            common::get_setting,
            common::set_setting,
            common::get_close_to_tray,
            common::set_close_to_tray,
            common::get_autostart,
            common::set_autostart,
            common::choose_file,
            common::get_favorites,
            common::toggle_favorite,
            // 许可证状态（只读）
            license::license_status,
            // WebView2 预检安装
            webview2::download_webview2,
            webview2::install_webview2,
            // 软件安装（任务队列）
            software::list_software,
            software::install_batch,
            software::queue_install,
            software::cancel_install,
            software::pause_tasks,
            software::resume_tasks,
            software::terminate_tasks,
            software::get_queue_state,
            software::set_task_complete_action,
            software::update_software_list,
            // 任务选项（执行模式 / 并发 / 限速）
            taskopts::get_task_opts,
            taskopts::set_task_opts,
            // 退出确认（任务运行时前端确认后真退出）
            force_exit,
            // 卸载与残留
            uninstall::list_installed,
            uninstall::scan_residual,
            uninstall::remove_residual,
            // 硬件
            hardware::hw_info,
            hardware::hw_usage,
            hardware::cpu_burn_start,
            hardware::cpu_burn_stop,
            hardware::disk_speed,
            hardware::gpu_burn_start,
            hardware::gpu_burn_stop,
            hardware::mem_test_start,
            // 系统监控：实时占用 / 进程 / 网络诊断
            sysmon::sys_usage,
            sysmon::list_processes,
            sysmon::kill_pid,
            sysmon::net_adapters,
            sysmon::ping_host,
            sysmon::dns_lookup,
            sysmon::flush_dns,
            // 磁盘空间分析
            diskspace::disk_overview,
            diskspace::dir_usage,
            diskspace::scan_large_files,
            diskspace::cancel_scan,
            diskspace::delete_space_item,
            diskspace::reveal_path,
            diskspace::empty_dir_contents,
            diskspace::scan_duplicates,
            // 批量任务模板
            templates::list_templates,
            templates::load_template,
            templates::save_template,
            templates::delete_template,
            templates::duplicate_template,
            templates::export_template_markdown,
            // 快捷工具：端口 / 系统入口 / 定时关机
            sysquick::list_ports,
            sysquick::open_system_tool,
            sysquick::schedule_shutdown,
            sysquick::cancel_shutdown,
            // 系统维护：回收站 / 电源计划 / 运行时长
            maintenance::empty_recycle_bin,
            maintenance::list_power_plans,
            maintenance::set_power_plan,
            maintenance::uptime_info,
            maintenance::install_precheck,
            netclient::test_proxy,
            // 系统工具
            systools::list_startup,
            systools::list_disabled_startup,
            systools::toggle_startup,
            systools::list_services,
            systools::set_service,
            systools::read_hosts,
            systools::write_hosts,
            systools::parse_hosts,
            systools::list_hosts_backups,
            systools::read_hosts_backup,
            systools::list_runtime_libs,
            systools::install_runtime,
            systools::list_uwp,
            systools::remove_uwp,
            systools::get_env_vars,
            systools::set_env_var,
            // 备份
            backup::create_backup,
            backup::list_backups,
            backup::restore_backup,
            backup::export_config,
            backup::import_config,
            // 开机自动备份软件清单
            backup::get_auto_backup_inventory,
            backup::set_auto_backup_inventory,
            // 垃圾清理与缓存
            cache::scan_junk,
            cache::clean_junk,
            cache::cache_stats,
            cache::clean_cache,
            // 缓存精细管理
            cache::cache_files,
            cache::cache_delete,
            // 日志
            logs::list_logs,
            logs::read_log,
            logs::open_log_dir,
            logs::clear_logs,
            // 日志保留策略
            logs::get_log_retention,
            logs::set_log_retention,
            // 引导壳多模式
            launcher::get_launch_mode,
            launcher::inst_choose_dir,
            launcher::inst_disk_free,
            launcher::inst_start,
            launcher::inst_cancel,
            launcher::inst_finish,
            launcher::unins_start,
            launcher::relaunch_repair,
            launcher::relaunch_as_admin,
            launcher::relaunch_portable,
            launcher::repair_start,
            launcher::inst_detect_existing,
            // 进程检测与关闭
            uninstall::process_running,
            uninstall::kill_process,
            uninstall::run_uninstaller,
            uninstall::measure_app_sizes,
            inventory::save_inventory,
            inventory::list_inventories,
            inventory::load_inventory,
            inventory::delete_inventory,
            inventory::rename_inventory,
            inventory::open_path,
            appicon::app_icon,
            appicon::prefetch_icons,
            dirs::set_cache_dir,
            // 磁盘 SMART / 温度
            hardware::disk_smart,
            hardware::cpu_temp,
            hardware::gpu_temp,
            // 驱动清单
            drivers::driver_list,
            drivers::export_driver_list,
            // 更新检查
            updater::check_update,
        ])
        .setup(|app| {
            if launcher::is_main_mode() {
                build_tray(app.handle())?;
                if let Some(win) = app.get_webview_window("main") {
                    restore_win_bounds(&win);
                }
                // 启动自动维护（后台线程）：日志按保留天数清理 + 开机自动备份软件清单
                logs::startup_retention();
                backup::startup_auto_inventory();
            } else if launcher::detect_launch_mode() == launcher::LaunchMode::Repair {
                // 修复模式：紧凑单卡窗口 720x520 禁 resize
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_min_size(Some(LogicalSize::new(400.0, 400.0)));
                    let _ = win.set_size(LogicalSize::new(720.0, 520.0));
                    let _ = win.set_resizable(false);
                }
            } else {
                // 安装向导 / 卸载器：同一窗口，调整为 780x540 轻量尺寸
                // （先放宽 minWidth/minHeight 980/640 的钳制）
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_min_size(Some(LogicalSize::new(400.0, 400.0)));
                    let _ = win.set_size(LogicalSize::new(780.0, 540.0));
                    let _ = win.set_resizable(false);
                }
            }
            // 前端就绪后统一显示窗口（消除启动白屏闪烁）
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if window.label() != "main" {
                    return;
                }
                // 安装/卸载模式：直接放行关闭，不走托盘隐藏与边界记忆
                if !launcher::is_main_mode() {
                    return;
                }
                // 无论是隐藏到托盘还是真退出，先记录窗口边界
                if let Some(win) = window.app_handle().get_webview_window("main") {
                    save_win_bounds(&win);
                }
                if common::close_to_tray_enabled() {
                    // 默认：点关闭 = 隐藏到托盘
                    api.prevent_close();
                    let _ = window.hide();
                    if !TRAY_HINT_EMITTED.swap(true, Ordering::Relaxed) {
                        let _ = window.emit("tray-hided", ());
                    }
                }
                // 关闭到托盘未开启：放行关闭请求，窗口销毁后应用退出
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
