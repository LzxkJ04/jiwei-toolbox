import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { toast } from "./toast";

/* ============ 类型定义（与 Rust 后端契约对应） ============ */

export interface Dirs {
  program: string;
  /** 用户数据目录（双路径模型；后端由其派生 config/backup/cache/log） */
  data: string;
  config: string;
  backup: string;
  cache: string;
  log: string;
}

export interface EnvStatus {
  is_admin: boolean;
  has_webview2: boolean;
}

/** 许可证页：后端 LicenseStatus 使用 camelCase */
export interface LicenseStatus {
  osEdition: string;
  osVersion: string;
  statusText: string;
  partialKey: string;
  productId: string;
  licensed: boolean;
  /** 完整版本号 */
  build?: string;
  /** EditionID，如 Professional */
  channel?: string;
  /** 系统安装日期 yyyy-MM-dd */
  installDate?: string;
  /** 宽限期剩余分钟（仅宽限状态有意义） */
  graceMinutes?: number;
}

export interface SoftwareItem {
  id: string;
  name: string;
  category: string;
  url: string;
  sha256: string;
  silentArgs: string;
  type: string;
  notes: string;
  /** 一句话简介 */
  description: string;
  /** 估算安装包/安装体积（MB） */
  size_mb: number;
  /** "free" | "open" | "commercial" */
  license: string;
  /** 官网 */
  homepage: string;
  /** 更新日志 */
  changelog: ChangelogEntry[];
  /** 依赖的其他软件 id */
  depends: string[];
}

export interface InstalledApp {
  name: string;
  version: string;
  publisher: string;
  uninstaller: string;
  key: string;
  size_estimate: number;
  install_location?: string;
}

export interface ResidualItem {
  kind: string; // "file" | "dir" | "reg"
  target: string;
}

export interface ChangelogEntry {
  version: string;
  date: string;
  notes: string;
}

export interface HwMemoryModule {
  manufacturer: string;
  capacity_gb: number;
  speed: number;
  /** 实际运行频率，未知为 0 */
  configured_mhz?: number;
  /** 插槽，如 Controller0-ChannelA-DIMM1 */
  slot?: string;
  bank?: string;
  part_number?: string;
  /** DIMM / SO-DIMM，未知为空 */
  form_factor?: string;
  /** 位宽，未知为 0 */
  data_width?: number;
  /** DDR4 / DDR5，未知为空 */
  type?: string;
}

export interface HwCpu {
  name: string;
  cores: number;
  threads: number;
  /** 最高睿频 / 标称频率 MHz */
  mhz?: number;
  current_mhz?: number;
  l2_kb?: number;
  l3_kb?: number;
  vendor?: string;
  arch?: string;
  socket?: string;
  /** null 表示系统没报告，不是「未开启」 */
  virtualization?: boolean | null;
}

export interface HwGpu {
  name: string;
  /** 显存 MB；0 表示未知 */
  vram: number;
  driver?: string;
  driver_date?: string;
  vendor?: string;
  width?: number;
  height?: number;
  refresh?: number;
  /** 远程/虚拟显示器，没有真实显存 */
  virtual?: boolean;
}

export interface HwDisk {
  model: string;
  size: number;
  media?: string;
  bus?: string;
  health?: string;
  serial?: string;
  firmware?: string;
}

export interface HwVolume {
  mount: string;
  total: number;
  free: number;
}

export interface HwOs {
  caption?: string;
  version?: string;
  build?: string;
  arch?: string;
}

export interface HwInfo {
  cpu: HwCpu;
  memory_total: number;
  /** 内存条明细（后端字段 memoryModules） */
  memoryModules?: HwMemoryModule[];
  board: string;
  motherboard?: { manufacturer?: string; product?: string; version?: string; serial?: string };
  bios?: { vendor?: string; version?: string; date?: string };
  os?: HwOs;
  gpus: HwGpu[];
  disks: HwDisk[];
  volumes?: HwVolume[];
}

export interface HwUsage {
  cpu_pct: number;
  mem_used: number;
  mem_total: number;
}

/** 系统监控：实时占用 + 网络速率 */
export interface SysUsage {
  cpu_pct: number;
  /** 每个逻辑核心的占用率，顺序与系统 CPU 编号一致 */
  per_core?: number[];
  mem_used: number;
  mem_total: number;
  swap_used: number;
  swap_total: number;
  /** 每秒接收字节 */
  down_bps: number;
  /** 每秒发送字节 */
  up_bps: number;
  processes: number;
  /** 系统已运行秒数 */
  uptime: number;
}

export interface ProcRow {
  pid: number;
  name: string;
  /** 占系统总 CPU 的百分比 */
  cpu: number;
  mem: number;
  exe: string;
  cmd: string;
}

export interface NetAdapter {
  alias: string;
  ipv4: string;
  gateway: string;
  dns: string;
  mac: string;
  speed: string;
}

export interface PingResult {
  ok: boolean;
  sent: number;
  lost: number;
  loss_pct: number;
  min: number | null;
  avg: number | null;
  max: number | null;
}

export interface SpaceEntry {
  name: string;
  path: string;
  size: number;
  /** "dir" | "file" */
  kind: string;
  files: number;
}

/** 内容完全相同的文件分组 */
export interface DuplicateGroup {
  hash: string;
  size: number;
  paths: string[];
  /** 除保留一份外可回收的字节数 */
  wasted: number;
}

export interface DiskOverview {
  mount: string;
  name: string;
  fs: string;
  total: number;
  free: number;
  used: number;
}

export interface DiskSpeed {
  read_mbps: number;
  write_mbps: number;
}

export interface StartupItem {
  name: string;
  command: string;
  location: string;
}

/** 已被本工具禁用、可从备份还原的启动项 */
export interface DisabledStartup {
  name: string;
  command: string;
  /** hkcu-run / hklm-run / hklm-run-wow / startup-folder-user / startup-folder-all */
  source: string;
  disabled_at: number;
}

export interface ServiceItem {
  name: string;
  display: string;
  status: string;
  start_type: string;
}

export interface JunkItem {
  path: string;
  size: number;
  kind: string;
}

export interface EnvVar {
  name: string;
  value: string;
}

export interface BackupItem {
  file: string;
  size: number;
  time: string;
}

export interface CacheStats {
  dirs: { name: string; size: number }[];
  total: number;
}

export interface InstallProgress {
  id: string;
  stage: string;
  pct: number;
  msg: string;
}

export interface InstallSummary {
  total: number;
  success: string[];
  failed: { id: string; reason: string }[];
  skipped: number;
}

export interface UpdateResult {
  count: number;
  source: string;
}

export interface GpuLoad {
  pct: number;
  fps: number;
  error?: string;
}

export interface MemTestProgress {
  pct: number;
  errors: number;
}

export interface MemTestResult {
  errors: number;
  blocks_total: number;
  seconds: number;
}

export interface RuntimeLib {
  id: string;
  name: string;
  installed: boolean;
  version: string;
  download_url: string;
}

export interface UwpApp {
  name: string;
  package: string;
  version: string;
}

export interface KillResult {
  killed: number[];
  failed: number[];
}

export interface DiskSmartInfo {
  model: string;
  serial: string;
  bus_type: string;
  media: string;
  /** OperationalStatus 原文，如 OK */
  status?: string;
  /** HealthStatus 原文（Healthy / Warning / Unhealthy）；兼容旧的数字健康度 */
  health?: string | number | null;
  /** 摄氏温度，可能缺失 */
  temperature_c?: number | null;
  /** 通电小时数，可能缺失 */
  power_on_hours?: number | null;
  /** 磨损百分比，可能缺失 */
  wear_pct?: number | null;
}

export interface DriverInfo {
  device_name: string;
  version: string;
  provider: string;
  date: string;
  inf_name: string;
}

export interface DriverExportResult {
  file: string;
  count: number;
}

export interface UpdateInfo {
  current: string;
  latest: string;
  has_update: boolean;
  /** 下载页地址，可能为空 */
  url?: string;
  /** 更新说明，可能为空 */
  notes?: string;
}

/* ============ 通用封装 ============ */

/** 带统一错误 toast 的安全调用；失败返回 null */
export async function tryCall<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  try {
    return await invoke<T>(cmd, args as any);
  } catch (err) {
    console.error(`[tauri] ${cmd} 调用失败:`, err);
    toast(`操作失败（${cmd}）：${String(err)}`, "error");
    return null;
  }
}

/** 注册事件监听，返回取消函数 */
export function onInstallProgress(handler: (p: InstallProgress) => void): Promise<UnlistenFn> {
  return listen<InstallProgress>("install-progress", (e) => handler(e.payload));
}

export function onCpuLoad(handler: (data: { pct: number }) => void): Promise<UnlistenFn> {
  return listen<{ pct: number }>("cpu-load", (e) => handler(e.payload));
}

export function onGpuLoad(handler: (data: GpuLoad) => void): Promise<UnlistenFn> {
  return listen<GpuLoad>("gpu-load", (e) => handler(e.payload));
}

export function onMemTestProgress(handler: (data: MemTestProgress) => void): Promise<UnlistenFn> {
  return listen<MemTestProgress>("mem-test-progress", (e) => handler(e.payload));
}

export function onMemTestDone(handler: (data: MemTestResult) => void): Promise<UnlistenFn> {
  return listen<MemTestResult>("mem-test-done", (e) => handler(e.payload));
}

export function onRuntimeProgress(
  handler: (data: { id: string; stage: string; pct: number }) => void
): Promise<UnlistenFn> {
  return listen<{ id: string; stage: string; pct: number }>("runtime-progress", (e) => handler(e.payload));
}

/** 批量安装完成汇总 */
export function onInstallSummary(handler: (data: InstallSummary) => void): Promise<UnlistenFn> {
  return listen<InstallSummary>("install-summary", (e) => handler(e.payload));
}

/** 托盘菜单触发的页面跳转 */
export function onNavigate(handler: (data: { page: string }) => void): Promise<UnlistenFn> {
  return listen<{ page: string }>("navigate", (e) => handler(e.payload));
}

/** 首次最小化到托盘提示 */
export function onTrayHided(handler: () => void): Promise<UnlistenFn> {
  return listen("tray-hided", () => handler());
}

/* ============ 命令包装 ============ */

// —— 环境 / 目录 / 设置
export const getDirs = () => tryCall<Dirs>("get_dirs");
export const getEnvStatus = () => tryCall<EnvStatus>("env_status");
export const downloadWebview2 = () => tryCall<string>("download_webview2");
export const installWebview2 = () => tryCall<boolean>("install_webview2");
export const getSetting = (key: string) => tryCall<string>("get_setting", { key });
export const setSetting = (key: string, value: string) => tryCall<null>("set_setting", { key, value });
export const getAutostart = () => tryCall<boolean>("get_autostart");
export const setAutostart = (enable: boolean) => tryCall<null>("set_autostart", { enable });
export const getCloseToTray = () => tryCall<boolean>("get_close_to_tray");
export const setCloseToTray = (enable: boolean) => tryCall<null>("set_close_to_tray", { enable });
export const getFavorites = () => tryCall<string[]>("get_favorites");
/** 切换收藏（已收藏则取消），返回更新后的收藏列表 */
export const toggleFavorite = (id: string) => tryCall<string[]>("toggle_favorite", { id });

// —— 软件安装
export const listSoftware = () => tryCall<SoftwareItem[]>("list_software");
export const installBatch = (ids: string[]) => tryCall<null>("install_batch", { ids });
export const cancelInstall = () => tryCall<null>("cancel_install");
/** 从远程源更新软件清单；失败抛错（tryCall 返回 null） */
export const updateSoftwareList = () => tryCall<UpdateResult>("update_software_list");

// —— 任务队列控制
export interface QueueStateInfo {
  running: boolean;
  paused: boolean;
  currentId: string;
  totalPct: number;
  total: number;
  done: number;
}
export const pauseTasks = () => tryCall<null>("pause_tasks");
export const resumeTasks = () => tryCall<null>("resume_tasks");
export const terminateTasks = (deleteCache: boolean) =>
  tryCall<null>("terminate_tasks", { deleteCache });
export const getQueueState = () => tryCall<QueueStateInfo>("get_queue_state");
/** 完成动作持久化：none | notify | log | shutdown | hibernate */
export const setTaskCompleteAction = (action: string) =>
  tryCall<null>("set_task_complete_action", { action });
/** 加入安装队列（后台串行，进度经 install-progress / queue-state 推送） */
export const queueInstall = (ids: string[]) => tryCall<null>("queue_install", { ids });
/** 真正退出（先终止任务队列） */
export const forceExit = () => invoke<null>("force_exit");

/** 队列运行状态推送 */
export function onQueueState(handler: (s: QueueStateInfo) => void): Promise<UnlistenFn> {
  return listen<QueueStateInfo>("queue-state", (e) => handler(e.payload));
}
/** 完成动作为"弹出通知"时后端推送 */
export function onTasksDone(handler: () => void): Promise<UnlistenFn> {
  return listen("tasks-done", () => handler());
}
/** 托盘点击退出但任务仍在执行：前端弹确认窗 */
export function onExitConfirmRequest(handler: () => void): Promise<UnlistenFn> {
  return listen("exit-confirm-request", () => handler());
}
/** 托盘操作结果通知（如清理缓存完成） */
export function onTrayNotify(handler: (msg: string) => void): Promise<UnlistenFn> {
  return listen<string | null>("tray-notify", (e) => handler(e.payload ?? ""));
}

// —— 卸载
export const listInstalled = () => tryCall<InstalledApp[]>("list_installed");
export const scanResidual = (name: string) => tryCall<ResidualItem[]>("scan_residual", { name });
export const removeResidual = (items: ResidualItem[]) => tryCall<null>("remove_residual", { items });
/** 启动枚举出来的卸载程序；只接受 msiexec /X 或已存在的 exe 路径 */
export const runUninstaller = (command: string) => invoke<string>("run_uninstaller", { command });
/** 补算体积：返回 [注册表键, 字节] 对，只包含 EstimatedSize 为 0 且能测到目录的项 */
export const measureAppSizes = () => invoke<[string, number][]>("measure_app_sizes").catch(() => []);
/** 按名称匹配正在运行的进程，返回 PID 列表（无匹配为空数组；调用失败返回 null，视为"未知"） */
export const processRunning = (name: string) => tryCall<number[]>("process_running", { name });
/** 结束指定进程，返回成功/失败 PID 分组 */
export const killProcess = (pids: number[]) => tryCall<KillResult>("kill_process", { pids });

// —— 硬件
export const hwInfo = () => tryCall<HwInfo>("hw_info");
/** 实时占用。轮询用，失败不弹 toast */
export const hwUsage = () => invoke<HwUsage>("hw_usage").catch(() => null);

/* ---- 系统监控 ---- */

/** 实时占用与网络速率。轮询用，失败不弹 toast */
export const sysUsage = () => invoke<SysUsage>("sys_usage").catch(() => null);
export const listProcesses = () => tryCall<ProcRow[]>("list_processes");
export const killPid = (pid: number) => invoke<null>("kill_pid", { pid });
export const netAdapters = () => tryCall<NetAdapter[]>("net_adapters");
export const pingHost = (host: string) => invoke<PingResult>("ping_host", { host });
export const dnsLookup = (host: string) => invoke<string[]>("dns_lookup", { host });
export const flushDns = () => invoke<string>("flush_dns");

/* ---- 磁盘空间 ---- */

export const diskOverview = () => tryCall<DiskOverview[]>("disk_overview");
export const dirUsage = (path: string, limit = 40) =>
  invoke<SpaceEntry[]>("dir_usage", { path, limit });
export const scanLargeFiles = (path: string, minMb: number, limit = 200) =>
  invoke<SpaceEntry[]>("scan_large_files", { path, minMb, limit });
export const cancelScan = () => invoke<null>("cancel_scan").catch(() => null);
export const deleteSpaceItem = (path: string) => invoke<null>("delete_space_item", { path });
export const revealPath = (path: string) => invoke<null>("reveal_path", { path });
export const emptyDirContents = (path: string) => invoke<number>("empty_dir_contents", { path });
/** 扫描重复文件；先按大小分桶再算 SHA-256 */
export const scanDuplicates = (path: string, minKb = 1) =>
  invoke<DuplicateGroup[]>("scan_duplicates", { path, minKb });

/* ---- 快捷工具 ---- */

export interface PortRow {
  proto: string;
  addr: string;
  port: number;
  pid: number;
  name: string;
}

export const listPorts = () => tryCall<PortRow[]>("list_ports");
export const openSystemTool = (id: string) => invoke<string>("open_system_tool", { id });
export const scheduleShutdown = (seconds: number, mode: string) =>
  invoke<null>("schedule_shutdown", { seconds, mode });
export const cancelShutdown = () => invoke<null>("cancel_shutdown");

/* ---- 系统维护 ---- */

export interface PowerPlan {
  id: string;
  name: string;
  active: boolean;
}

export interface UptimeInfo {
  boot_ts: number;
  uptime: number;
  boot_seconds: number;
}

export const emptyRecycleBin = () => invoke<null>("empty_recycle_bin");
export const listPowerPlans = () => tryCall<PowerPlan[]>("list_power_plans");
export const setPowerPlan = (id: string) => invoke<null>("set_power_plan", { id });
export const uptimeInfo = () => tryCall<UptimeInfo>("uptime_info");

/** 安装前环境预检项 */
export interface PrecheckItem {
  id: string;
  label: string;
  /** "ok" | "warn" | "fail" */
  level: string;
  detail: string;
  hint: string;
}

export const installPrecheck = (programDir: string, dataDir: string) =>
  tryCall<PrecheckItem[]>("install_precheck", { programDir, dataDir });

/** 代理连通性测试 */
export interface ProxyTest {
  ok: boolean;
  proxy: string;
  from_system: boolean;
  elapsed_ms: number;
  message: string;
}

export const testProxy = () => invoke<ProxyTest>("test_proxy");



export const cpuBurnStart = (seconds: number) => tryCall<null>("cpu_burn_start", { seconds });
export const cpuBurnStop = () => tryCall<null>("cpu_burn_stop");
export const diskSpeed = (path: string) => tryCall<DiskSpeed>("disk_speed", { path });
/** 只读查询 Windows 许可证状态（不含激活） */
export const licenseStatus = () => tryCall<LicenseStatus>("license_status");
export const gpuBurnStart = (seconds: number) => tryCall<null>("gpu_burn_start", { seconds });
export const gpuBurnStop = () => tryCall<null>("gpu_burn_stop");
export const memTestStart = () => tryCall<null>("mem_test_start");
/** 读取磁盘 SMART 健康信息；失败（无管理员权限/不支持 SMART）返回 null */
export const diskSmart = () => tryCall<DiskSmartInfo[]>("disk_smart");
/** CPU 温度（摄氏），传感器不可用返回 null */
export const cpuTemp = () => tryCall<number | null>("cpu_temp");
/** GPU 温度（摄氏），传感器不可用返回 null */
export const gpuTemp = () => tryCall<number | null>("gpu_temp");

// —— 驱动
/** 枚举本机驱动；失败返回 null */
export const driverList = () => tryCall<DriverInfo[]>("driver_list");
/** 导出驱动清单 CSV，返回文件路径与条数 */
export const exportDriverList = () => tryCall<DriverExportResult>("export_driver_list");

// —— 更新
/** 检查应用更新；失败返回 null */
export const checkUpdate = () => tryCall<UpdateInfo>("check_update");

// —— 系统工具
export const listStartup = () => tryCall<StartupItem[]>("list_startup");
export const listDisabledStartup = () => tryCall<DisabledStartup[]>("list_disabled_startup");
export const toggleStartup = (name: string, enable: boolean) =>
  tryCall<null>("toggle_startup", { name, enable });
export const listServices = () => tryCall<ServiceItem[]>("list_services");
export const setService = (name: string, action: string) =>
  tryCall<null>("set_service", { name, action });
export const readHosts = () => tryCall<string>("read_hosts");
export const writeHosts = (content: string) => tryCall<null>("write_hosts", { content });

/** hosts 结构化条目 */
export interface HostEntry {
  ip: string;
  domains: string[];
  /** 以 # 注释掉的视为停用 */
  enabled: boolean;
  /** 系统自带示例行（localhost 映射），界面默认折叠 */
  builtin: boolean;
}

export const parseHosts = () => tryCall<HostEntry[]>("parse_hosts");
export const listHostsBackups = () => tryCall<string[]>("list_hosts_backups");
export const readHostsBackup = (file: string) => tryCall<string>("read_hosts_backup", { file });
export const scanJunk = () => tryCall<JunkItem[]>("scan_junk");
export const cleanJunk = (paths: string[]) => tryCall<null>("clean_junk", { paths });
export const getEnvVars = (scope: "user" | "system") => tryCall<EnvVar[]>("get_env_vars", { scope });
export const setEnvVar = (scope: "user" | "system", name: string, value: string, isDelete: boolean) =>
  tryCall<null>("set_env_var", { scope, name, value, isDelete });
export const listRuntimeLibs = () => tryCall<RuntimeLib[]>("list_runtime_libs");
export const installRuntime = (id: string) => tryCall<null>("install_runtime", { id });
export const listUwp = () => tryCall<UwpApp[]>("list_uwp");
export const removeUwp = (pkg: string) => tryCall<null>("remove_uwp", { package: pkg });
export const exportConfig = () => tryCall<{ file: string }>("export_config");
export const importConfig = (file: string) => tryCall<null>("import_config", { file });

// —— 备份 / 缓存
export const createBackup = (note: string) => tryCall<{ file: string }>("create_backup", { note });
export const listBackups = () => tryCall<BackupItem[]>("list_backups");
export const restoreBackup = (file: string) => tryCall<null>("restore_backup", { file });
export const cacheStats = () => tryCall<CacheStats>("cache_stats");
export const cleanCache = () => tryCall<null>("clean_cache");

export interface CacheFile {
  name: string;
  path: string;
  size: number;
  /** 修改时间（秒级时间戳） */
  mtime: number;
  /** "download" | "temp" | "log" */
  kind: string;
}
/** 缓存目录内文件明细（供缓存管理弹窗） */
export const cacheFiles = () => tryCall<CacheFile[]>("cache_files");
/** 按文件名删除缓存文件 */
export const cacheDelete = (names: string[]) => tryCall<null>("cache_delete", { names });

// —— 任务默认选项 / 自动清理
export interface TaskOpts {
  /** "serial" | "parallel" */
  mode: string;
  /** 下载并发数 1~5 */
  concurrency: number;
  /** 下载限速 KB/s，0 = 不限速 */
  speed_limit_kb: number;
}
export const getTaskOpts = () => tryCall<TaskOpts>("get_task_opts");
export const setTaskOpts = (mode: string, concurrency: number, speed_limit_kb: number) =>
  tryCall<null>("set_task_opts", { mode, concurrency, speed_limit_kb });
/** 启动时自动生成软件清单简易备份 */
export const getAutoBackupInventory = () => tryCall<boolean>("get_auto_backup_inventory");
export const setAutoBackupInventory = (enable: boolean) =>
  tryCall<null>("set_auto_backup_inventory", { enable });
/** 日志自动清理周期（天），0 = 关闭 */
export const setLogRetention = (days: number) => tryCall<null>("set_log_retention", { days });

// —— 日志
export const listLogs = () => tryCall<string[]>("list_logs");
export const readLog = (file: string) => tryCall<string>("read_log", { file });
export const openLogDir = () => tryCall<null>("open_log_dir");
/** 清空日志目录（删除全部日志文件），返回删除的文件数 */
export const clearLogs = () => tryCall<number>("clear_logs");

/* ============ 引导壳（安装器 / 卸载器） ============ */

/** 启动模式；未接入后端（如纯浏览器调试）时回退为 main */
export async function getLaunchMode(): Promise<
  "main" | "installer" | "uninstaller" | "repair" | "unattended"
> {
  try {
    const m = await invoke<string>("get_launch_mode");
    return m === "installer" || m === "uninstaller" || m === "repair" || m === "unattended"
      ? (m as "installer" | "uninstaller" | "repair" | "unattended")
      : "main";
  } catch {
    return "main";
  }
}

// —— 安装器
/** 打开系统目录选择对话框，返回所选目录（取消返回空串/null） */
export const instChooseDir = () => tryCall<string>("inst_choose_dir");
/** 系统文件选择框；取消返回空字符串 */
export const chooseFile = (title: string, filterName = "", filterExt = "") =>
  invoke<string>("choose_file", { title, filterName, filterExt }).catch(() => "");
/** 查询路径所在盘剩余空间（字节；失败 0=未知） */
export const instDiskFree = (path: string) => tryCall<number>("inst_disk_free", { path });
/** 开始安装：WebView2 预装（按需）+ 拷贝程序文件 + 创建数据目录 + 静默安装逻辑包 */
export const instStart = (
  dir: string,
  dataDir: string,
  desktop: boolean,
  startMenu: boolean,
  admin: boolean,
  installWebv2: boolean
) => tryCall<null>("inst_start", { dir, dataDir, desktop, startMenu, admin, installWebv2 });
/** 取消安装（后端终止任务；仅拷贝阶段生效） */
export const instCancel = () => tryCall<null>("inst_cancel");
/** 完成安装：可选立即启动 / 打开配置目录，随后后端退出引导进程 */
export const instFinish = (run: boolean, openCfg: boolean) =>
  tryCall<null>("inst_finish", { run, openCfg });

export interface InstallerProgress {
  stage: "copy" | "logic" | "webview2" | "done";
  pct: number;
  msg: string;
}
export interface InstallerDone {
  ok: boolean;
  dir?: string;
  err?: string;
}
export function onInstallerProgress(handler: (p: InstallerProgress) => void): Promise<UnlistenFn> {
  return listen<InstallerProgress>("installer-progress", (e) => handler(e.payload));
}
export function onInstallerDone(handler: (d: InstallerDone) => void): Promise<UnlistenFn> {
  return listen<InstallerDone>("installer-done", (e) => handler(e.payload));
}

/** WebView2 预检下载/安装进度（依赖预检页直接调用 download/install 时使用） */
export function onWebview2Progress(
  handler: (p: { stage: string; pct: number }) => void
): Promise<UnlistenFn> {
  return listen<{ stage: string; pct: number }>("webview2-progress", (e) => handler(e.payload));
}

// —— 卸载器
/** 开始卸载；keep_data = 是否保留用户配置与备份数据 */
export const uninsStart = (keepData: boolean) => tryCall<null>("unins_start", { keepData });

export function onUninstallerProgress(
  handler: (p: { stage: string }) => void
): Promise<UnlistenFn> {
  return listen<{ stage: string }>("uninstaller-progress", (e) => handler(e.payload));
}
export function onUninstallerDone(handler: (d: { ok: boolean }) => void): Promise<UnlistenFn> {
  return listen<{ ok: boolean }>("uninstaller-done", (e) => handler(e.payload));
}

// —— 修复模式
/** 从主程序重启进入修复模式（后端 spawn 自身 --repair 后退出当前进程） */
export const relaunchRepair = () => tryCall<null>("relaunch_repair");
/** 开始修复：校验程序文件 / 重建 dirs.json / 修复快捷方式与文件关联 */
export const repairStart = () => tryCall<null>("repair_start");

export interface RepairProgress {
  stage: "check" | "dirs" | "shortcut" | "done";
  pct: number;
  msg: string;
}
export interface RepairDoneItem {
  item: string;
  /** "ok" | "repaired" | "warn" */
  status: string;
  detail: string;
}
export interface RepairDone {
  ok: boolean;
  items: RepairDoneItem[];
}
export function onRepairProgress(handler: (p: RepairProgress) => void): Promise<UnlistenFn> {
  return listen<RepairProgress>("repair-progress", (e) => handler(e.payload));
}
export function onRepairDone(handler: (d: RepairDone) => void): Promise<UnlistenFn> {
  return listen<RepairDone>("repair-done", (e) => handler(e.payload));
}

// —— 版本升级预填
export interface ExistingInstall {
  exists: boolean;
  program: string;
  data: string;
  oldVersion: string;
}
/** 检测已有安装（注册表 Dirs + 版本号），供安装向导预填 */
export const instDetectExisting = () => tryCall<ExistingInstall>("inst_detect_existing");

// —— 软件清单备份（后端持久化：data\备份导出\*.jwinv.json）——
export interface InventoryAppRef {
  name: string;
  version: string;
  publisher: string;
  install_location?: string;
}
export interface InventoryMeta {
  name: string;
  file: string;
  created: string;
  count: number;
  size: number;
  mtime: number;
}
export interface InventoryDoc {
  kind: string;
  name: string;
  created: string;
  apps: InventoryAppRef[];
}
export const saveInventory = (name: string, apps: InventoryAppRef[]) =>
  tryCall<{ file: string; count: number }>("save_inventory", { name, apps });
export const listInventories = () => tryCall<InventoryMeta[]>("list_inventories");
export const loadInventoryDoc = (name: string) => tryCall<InventoryDoc>("load_inventory", { name });
export const deleteInventory = (name: string) => tryCall<null>("delete_inventory", { name });
export const renameInventory = (oldName: string, newName: string) =>
  tryCall<null>("rename_inventory", { oldName, newName });

/** 用系统默认方式打开路径（目录=资源管理器，文件=关联程序） */
export const openPath = (path: string) => tryCall<null>("open_path", { path });

/** 软件图标 data URL（磁盘缓存离线可用；空串=失败，前端降级首字母） */
export const appIcon = (homepage: string) => tryCall<string>("app_icon", { homepage });
/** 后台批量预取图标（软件库加载后调用），进度经 icon-prefetch 事件推送 */
export const prefetchIcons = (homepages: string[]) =>
  tryCall<null>("prefetch_icons", { homepages });

/** 以 --portable 重启自身（便携模式，跳过安装向导） */
export const relaunchPortable = () => tryCall<null>("relaunch_portable");
/** 修改缓存目录（log 派生），写回 dirs.json 并创建目录 */
export const setCacheDir = (newCache: string) => tryCall<null>("set_cache_dir", { newCache });
