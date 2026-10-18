//! 硬件信息、CPU 烤机、磁盘测速

use serde::Serialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::Emitter;
use winreg::enums::HKEY_LOCAL_MACHINE;
use winreg::RegKey;

static BURN_RUN: AtomicBool = AtomicBool::new(false);
static BURN_WORKERS: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());
static BURN_MONITOR: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

#[derive(Serialize)]
pub struct DiskSpeed {
    pub read_mbps: f64,
    pub write_mbps: f64,
}

// ---------- PowerShell WMI 辅助 ----------

fn ps_json(script: &str) -> Option<serde_json::Value> {
    let full = format!("[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; {}", script);
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &full]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&text).ok()
}

/// ConvertTo-Json 单元素时输出对象而非数组 → 统一为数组
fn as_array(v: Option<serde_json::Value>) -> Vec<serde_json::Value> {
    match v {
        Some(serde_json::Value::Array(a)) => a,
        Some(x) => vec![x],
        None => vec![],
    }
}

fn jstr(v: &serde_json::Value, key: &str, default: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(default)
        .to_string()
}

fn jnum(v: &serde_json::Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(default)
}

/// 物理内存条列表（Win32_PhysicalMemory）
fn memory_modules_ps() -> Vec<serde_json::Value> {
    as_array(ps_json(
        r#"@(Get-CimInstance Win32_PhysicalMemory | Select-Object Manufacturer,Capacity,Speed) | ConvertTo-Json -Compress"#,
    ))
    .iter()
    .map(|m| {
        json!({
            "manufacturer": jstr(m, "Manufacturer", "未知"),
            "capacity_gb": (jnum(m, "Capacity", 0.0) / 1073741824.0 * 10.0).round() / 10.0,
            "speed": jnum(m, "Speed", 0.0) as u64,
        })
    })
    .collect()
}

/// 物理磁盘（Win32_DiskDrive）+ 卷（sysinfo 挂载点）
fn disks_info() -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let disks: Vec<serde_json::Value> = as_array(ps_json(
        r#"@(Get-CimInstance Win32_DiskDrive | Select-Object Model,Size) | ConvertTo-Json -Compress"#,
    ))
    .iter()
    .map(|d| {
        // Win32_DiskDrive.Size 可能是字符串数字
        let size_raw = d.get("Size").cloned().unwrap_or(serde_json::Value::Null);
        let size_b = match &size_raw {
            serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
            serde_json::Value::String(st) => st.trim().parse::<f64>().unwrap_or(0.0),
            _ => 0.0,
        };
        json!({
            "model": jstr(d, "Model", "未知"),
            "capacity_gb": (size_b / 1073741824.0).round() as u64,
            "size": size_b as u64,
        })
    })
    .collect();

    let vol_disks = sysinfo::Disks::new_with_refreshed_list();
    let volumes: Vec<serde_json::Value> = vol_disks
        .list()
        .iter()
        .map(|d| {
            json!({
                "mount": d.mount_point().to_string_lossy(),
                "total_gb": d.total_space() / 1073741824,
                "free_gb": d.available_space() / 1073741824,
            })
        })
        .collect();
    (disks, volumes)
}

/// BIOS / 主板（注册表 HARDWARE\DESCRIPTION\System\BIOS）
fn bios_info() -> (String, String, String, String) {
    let (mut mfr, mut product, mut ver, mut date) =
        (String::new(), String::new(), String::new(), String::new());
    if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"HARDWARE\DESCRIPTION\System\BIOS") {
        mfr = k.get_value::<String, _>("SystemManufacturer").unwrap_or_default();
        product = k.get_value::<String, _>("SystemProductName").unwrap_or_default();
        ver = k.get_value::<String, _>("BIOSVersion").unwrap_or_default();
        date = k.get_value::<String, _>("BIOSReleaseDate").unwrap_or_default();
    }
    (mfr, product, ver, date)
}

/// AdapterRAM（字节）→ MB。WMI 用 uint32，大于 4GB 的显卡常返回 -1 或溢出值，此时记 0（未知）。
fn vram_mb(raw: f64) -> u64 {
    const MAX_BYTES: f64 = 64.0 * 1024.0 * 1024.0 * 1024.0;
    if raw <= 0.0 || raw > MAX_BYTES {
        return 0;
    }
    (raw / 1_048_576.0).round() as u64
}

/// 显卡列表（WMI Win32_VideoController：名称 + 显存 MB）
fn gpu_list() -> Vec<serde_json::Value> {
    let arr = as_array(ps_json(
        r#"@(Get-CimInstance Win32_VideoController | Select-Object Name,AdapterRAM) | ConvertTo-Json -Compress"#,
    ));
    let mut out: Vec<serde_json::Value> = arr
        .iter()
        .map(|g| {
            let name = jstr(g, "Name", "");
            let vram = vram_mb(jnum(g, "AdapterRAM", 0.0));
            json!({ "name": if name.trim().is_empty() { "未知显卡".into() } else { name }, "vram": vram })
        })
        .collect();
    // WMI 查询失败时回退：注册表 DriverDesc（无显存信息）
    if out.is_empty() {
        let base = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
        if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(base) {
            for sub in k.enum_keys().flatten() {
                if let Ok(sk) = k.open_subkey(&sub) {
                    if let Ok(desc) = sk.get_value::<String, _>("DriverDesc") {
                        if !desc.trim().is_empty() {
                            out.push(json!({ "name": desc, "vram": 0 }));
                        }
                    }
                }
            }
        }
    }
    // 去重
    out.dedup_by(|a, b| a["name"] == b["name"]);
    out
}

/// 硬件信息总览（CPU / 内存 / 主板 / BIOS / 显卡 / 磁盘）
#[tauri::command]
pub async fn hw_info() -> serde_json::Value {
    use sysinfo::System;
    let mut sys = System::new_all();
    sys.refresh_cpu_usage();

    let cpus = sys.cpus();
    let threads = cpus.len();
    let brand = cpus.first().map(|c| c.brand().trim().to_string()).unwrap_or_default();
    let cores = sys.physical_core_count().unwrap_or(threads);
    let memory_total = sys.total_memory();

    let (board_mfr, board_product, bios_ver, bios_date) = bios_info();
    let gpus = gpu_list();

    let mut memory_modules = memory_modules_ps();
    if memory_modules.is_empty() {
        memory_modules = vec![json!({
            "manufacturer": "未知",
            "capacity_gb": (memory_total as f64 / 1073741824.0 * 10.0).round() / 10.0,
            "speed": 0,
        })];
    }
    let (disks, volumes) = disks_info();

    let board = [board_mfr.trim(), board_product.trim()]
        .iter()
        .filter(|x| !x.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    json!({
        "cpu": { "name": brand, "cores": cores, "threads": threads },
        "memory_total": memory_total,
        "memoryModules": memory_modules,
        "motherboard": { "manufacturer": board_mfr, "product": board_product },
        "board": board,
        "bios": { "version": bios_ver, "date": bios_date },
        "gpus": gpus,
        "disks": disks,
        "volumes": volumes,
    })
}

// ---------- CPU 烤机 ----------

fn stop_burn() {
    BURN_RUN.store(false, Ordering::Relaxed);
    if let Ok(mut v) = BURN_WORKERS.lock() {
        for h in v.drain(..) {
            let _ = h.join();
        }
    }
    if let Ok(mut m) = BURN_MONITOR.lock() {
        if let Some(h) = m.take() {
            let _ = h.join();
        }
    }
}

/// 启动 CPU 烤机（多线程忙运算，seconds 秒后自动停止；每秒发 cpu-load {pct} 事件）
#[tauri::command]
pub fn cpu_burn_start(app: tauri::AppHandle, seconds: u64) {
    // 返回前先停旧任务
    stop_burn();
    BURN_RUN.store(true, Ordering::Relaxed);

    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    if let Ok(mut v) = BURN_WORKERS.lock() {
        for _ in 0..n {
            v.push(std::thread::spawn(|| {
                let mut x = 1.5f64;
                while BURN_RUN.load(Ordering::Relaxed) {
                    for _ in 0..50_000 {
                        x = x * 1.0000001 + 0.0000001;
                        if x > 1e9 {
                            x = 1.0;
                        }
                    }
                    std::hint::black_box(x);
                }
            }));
        }
    }

    // 监控线程：采样真实 CPU 负载率
    let monitor_app = app;
    if let Ok(mut m) = BURN_MONITOR.lock() {
        m.replace(std::thread::spawn(move || {
            use sysinfo::System;
            let mut sys = System::new_all();
            let start = Instant::now();
            while BURN_RUN.load(Ordering::Relaxed) && start.elapsed().as_secs() < seconds {
                std::thread::sleep(Duration::from_millis(1000));
                sys.refresh_cpu_usage();
                let pct = sys.global_cpu_usage();
                let _ = monitor_app.emit("cpu-load", json!({ "pct": pct }));
            }
            BURN_RUN.store(false, Ordering::Relaxed);
        }));
    }
}

/// 停止 CPU 烤机
#[tauri::command]
pub fn cpu_burn_stop() {
    stop_burn();
}

// ---------- 磁盘测速 ----------

/// 磁盘测速：写 512MB 临时文件测写速，读回测读速，测毕删除
#[tauri::command]
pub async fn disk_speed(path: String) -> Result<DiskSpeed, String> {
    use std::io::{Read, Write};

    // 兼容多种输入：目录 / 盘符 "C" / "C:" / "C:\"；无效时回退系统临时目录
    let mut dir = PathBuf::from(&path);
    if !dir.is_dir() {
        let p = path.trim();
        dir = if p.len() >= 2 && p.as_bytes()[1] == b':' {
            PathBuf::from(format!("{}\\", &p[..2]))
        } else {
            std::env::temp_dir()
        };
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法使用目录 {}: {}", dir.display(), e))?;
    let file_path = dir.join("jwspeed.tmp");

    let total: u64 = 512 * 1024 * 1024;
    // 4MB 伪随机块（避免被文件系统/压缩优化）
    let mut data = vec![0u8; 4 * 1024 * 1024];
    let mut seed = 0x12345678u32;
    for b in data.iter_mut() {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *b = (seed >> 24) as u8;
    }

    // 写测速
    let start = Instant::now();
    {
        let mut f = std::fs::File::create(&file_path).map_err(|e| format!("创建临时文件失败: {}", e))?;
        let mut written = 0u64;
        while written < total {
            let n = ((total - written) as usize).min(data.len());
            f.write_all(&data[..n]).map_err(|e| format!("写入失败: {}", e))?;
            written += n as u64;
        }
        f.sync_all().map_err(|e| format!("刷盘失败: {}", e))?;
    }
    let write_mbps = total as f64 / 1048576.0 / start.elapsed().as_secs_f64();

    // 读测速
    let start = Instant::now();
    {
        let mut f = std::fs::File::open(&file_path).map_err(|e| format!("读取失败: {}", e))?;
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        loop {
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => return Err(format!("读取失败: {}", e)),
            }
        }
    }
    let read_mbps = total as f64 / 1048576.0 / start.elapsed().as_secs_f64();

    let _ = std::fs::remove_file(&file_path);
    Ok(DiskSpeed { read_mbps, write_mbps })
}

// ---------- GPU 烤机（wgpu compute） ----------

/// 矩阵边长（256x256 f32，单次 dispatch ≈ 33.5 MFLOP）
const MAT_N: u32 = 256;
/// 每次 submit 重复 dispatch 的批次数（避免 CPU 提交开销成为瓶颈）
const GPU_BATCH: u32 = 64;

const GPU_SHADER: &str = r#"
const N: u32 = 256u;
@group(0) @binding(0) var<storage, read> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let row = gid.y;
    let col = gid.x;
    if (row >= N || col >= N) {
        return;
    }
    var sum = 0.0;
    for (var k: u32 = 0u; k < N; k = k + 1u) {
        sum = sum + a[row * N + k] * b[k * N + col];
    }
    out[row * N + col] = sum;
}
"#;

/// 极简 block_on（noop waker 忙等，仅用于初始化阶段的同步等待）
fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }
    let waker = Waker::from(Arc::new(NoopWake));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
}

impl GpuContext {
    fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| "未找到可用的 GPU 适配器".to_string())?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("jw-gpu-burn"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
        }, None))
        .map_err(|e| e.to_string())?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("jw-gpu-burn.wgsl"),
            source: wgpu::ShaderSource::Wgsl(GPU_SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("jw-gpu-burn-pipeline"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let elem = MAT_N as u64 * MAT_N as u64;
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let mk = |usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: elem * 4,
                usage,
                mapped_at_creation: false,
            })
        };
        let (buf_a, buf_b, buf_out) = (mk(usage), mk(usage), mk(wgpu::BufferUsages::STORAGE));

        // 填充输入数据（LCG 伪随机，避免全零被优化）
        let mut seed = 0x12345678u32;
        let mut data = vec![0u8; (elem * 4) as usize];
        for chunk in data.chunks_exact_mut(4) {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            chunk.copy_from_slice(&seed.to_ne_bytes());
        }
        queue.write_buffer(&buf_a, 0, &data);
        queue.write_buffer(&buf_b, 0, &data);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf_a.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: buf_b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: buf_out.as_entire_binding() },
            ],
            label: None,
        });

        Ok(GpuContext { device, queue, pipeline, bind_group })
    }
}

static GPU_RUN: AtomicBool = AtomicBool::new(false);
static GPU_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

fn stop_gpu_burn() {
    GPU_RUN.store(false, Ordering::Relaxed);
    if let Ok(mut t) = GPU_THREAD.lock() {
        if let Some(h) = t.take() {
            let _ = h.join();
        }
    }
}

/// 启动 GPU 烤机：compute 大矩阵乘法循环，seconds 秒后自动停止；
/// 每 500ms 发 gpu-load {pct, fps}（pct 恒 100，fps 为每秒迭代次数）
#[tauri::command]
pub fn gpu_burn_start(app: tauri::AppHandle, seconds: u64) -> Result<(), String> {
    stop_gpu_burn();
    GPU_RUN.store(true, Ordering::Relaxed);

    // 同步初始化，失败则上报并返回 Err
    let ctx = match GpuContext::new() {
        Ok(c) => c,
        Err(e) => {
            GPU_RUN.store(false, Ordering::Relaxed);
            let _ = app.emit("gpu-load", json!({ "error": "不支持" }));
            return Err(format!("当前环境不支持 wgpu: {}", e));
        }
    };

    if let Ok(mut t) = GPU_THREAD.lock() {
        t.replace(std::thread::spawn(move || {
            let start = Instant::now();
            let mut last_emit = Instant::now();
            let mut window_iters: u64 = 0;
            let mut window_start = Instant::now();
            while GPU_RUN.load(Ordering::Relaxed) && start.elapsed().as_secs() < seconds {
                let mut encoder = ctx
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: None,
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&ctx.pipeline);
                    pass.set_bind_group(0, &ctx.bind_group, &[]);
                    for _ in 0..GPU_BATCH {
                        pass.dispatch_workgroups(MAT_N / 16, MAT_N / 16, 1);
                    }
                }
                ctx.queue.submit(Some(encoder.finish()));
                let _ = ctx.device.poll(wgpu::Maintain::Wait);
                window_iters += GPU_BATCH as u64;

                if last_emit.elapsed() >= Duration::from_millis(500) {
                    let secs = window_start.elapsed().as_secs_f64().max(0.001);
                    let fps = (window_iters as f64 / secs).round() as u64;
                    window_iters = 0;
                    window_start = Instant::now();
                    last_emit = Instant::now();
                    let _ = app.emit("gpu-load", json!({ "pct": 100, "fps": fps }));
                }
            }
            GPU_RUN.store(false, Ordering::Relaxed);
        }));
    }
    Ok(())
}

/// 停止 GPU 烤机
#[tauri::command]
pub fn gpu_burn_stop() -> Result<(), String> {
    stop_gpu_burn();
    Ok(())
}

// ---------- 磁盘 SMART 健康 ----------

/// OperationalStatus 可能是字符串或数组，统一为字符串
fn jany(v: &serde_json::Value, key: &str, default: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        _ => default.to_string(),
    }
}

/// 取可选数值字段（WMI 侧缺失/为 null 时返回 None）
fn jopt(v: Option<&serde_json::Value>, key: &str) -> Option<f64> {
    v.and_then(|x| x.get(key)).and_then(|x| x.as_f64())
}

/// 磁盘 SMART 健康：Get-PhysicalDisk + Get-StorageReliabilityCounter 按 DeviceId 对齐；
/// 部分盘无可靠性数据（需管理员/不支持）时相应字段为 null
#[tauri::command]
pub async fn disk_smart() -> Result<Vec<serde_json::Value>, String> {
    let disks = as_array(ps_json(
        r#"@(Get-PhysicalDisk | Select-Object DeviceId,FriendlyName,SerialNumber,BusType,MediaType,HealthStatus,OperationalStatus) | ConvertTo-Json -Compress"#,
    ));
    if disks.is_empty() {
        return Err("无法读取磁盘信息（需要管理员权限或硬盘不支持）".into());
    }
    let rel = as_array(ps_json(
        r#"@(Get-PhysicalDisk | Get-StorageReliabilityCounter | Select-Object DeviceId,Temperature,PowerOnHours,Wear) | ConvertTo-Json -Compress"#,
    ));

    let out: Vec<serde_json::Value> = disks
        .iter()
        .map(|d| {
            let dev = jstr(d, "DeviceId", "");
            let r = rel.iter().find(|x| jstr(x, "DeviceId", "") == dev);
            json!({
                "model": jstr(d, "FriendlyName", "未知"),
                "serial": jstr(d, "SerialNumber", ""),
                "bus_type": jstr(d, "BusType", ""),
                "media": jstr(d, "MediaType", ""),
                "health": jstr(d, "HealthStatus", ""),
                "status": jany(d, "OperationalStatus", ""),
                "temperature_c": jopt(r, "Temperature"),
                "power_on_hours": jopt(r, "PowerOnHours").map(|v| v as u64),
                "wear_pct": jopt(r, "Wear"),
            })
        })
        .collect();
    Ok(out)
}

// ---------- 温度 ----------

/// 四舍五入到 0.1℃
fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// CPU 温度：sysinfo 温度组件（名含 CPU/Core 取均值）→ WMI 热区回退 → null
#[tauri::command]
pub async fn cpu_temp() -> Option<f64> {
    // sysinfo（Windows 上支持有限，取不到则走 WMI）
    let comps = sysinfo::Components::new_with_refreshed_list();
    let vals: Vec<f64> = comps
        .list()
        .iter()
        .filter(|c| {
            let l = c.label().to_lowercase();
            l.contains("cpu") || l.contains("core")
        })
        .filter_map(|c| c.temperature())
        .map(|t| t as f64)
        .filter(|t| *t > 0.0 && *t < 150.0)
        .collect();
    if !vals.is_empty() {
        let n = vals.len() as f64;
        return Some(round1(vals.iter().sum::<f64>() / n));
    }

    // WMI 热区回退：CurrentTemperature 为十分之一开尔文
    let zones = as_array(ps_json(
        r#"@(Get-CimInstance MSAcpi_ThermalZoneTemperature -Namespace root/wmi | Select-Object CurrentTemperature) | ConvertTo-Json -Compress"#,
    ));
    let vals: Vec<f64> = zones
        .iter()
        .filter_map(|z| z.get("CurrentTemperature")?.as_f64())
        .map(|k| k / 10.0 - 273.15)
        .filter(|t| *t > 0.0 && *t < 150.0)
        .collect();
    if vals.is_empty() {
        return None;
    }
    let n = vals.len() as f64;
    Some(round1(vals.iter().sum::<f64>() / n))
}

/// GPU 温度：nvidia-smi（PATH → System32 → NVSMI 安装目录），不可用返回 null
#[tauri::command]
pub async fn gpu_temp() -> Option<f64> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    for prog in [
        "nvidia-smi",
        r"C:\Windows\System32\nvidia-smi.exe",
        r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
    ] {
        let out = std::process::Command::new(prog)
            .args(["--query-gpu=temperature.gpu", "--format=csv,noheader,nounits"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Ok(out) = out {
            if out.status.success() {
                if let Ok(s) = String::from_utf8(out.stdout) {
                    if let Some(t) = s.lines().next().and_then(|l| l.trim().parse::<f64>().ok()) {
                        if t > 0.0 && t < 150.0 {
                            return Some(round1(t));
                        }
                    }
                }
            }
        }
    }
    None
}

// ---------- 内存测试 ----------

static MEM_TEST_RUN: AtomicBool = AtomicBool::new(false);

/// 启动内存测试：分配大块内存写入 0xAA/0x55/随机模式并读回校验；
/// 逐块发 mem-test-progress {pct, errors}，完成发 mem-test-done {errors, blocks_total, seconds}
#[tauri::command]
pub fn mem_test_start(app: tauri::AppHandle) -> Result<(), String> {
    if MEM_TEST_RUN.swap(true, Ordering::SeqCst) {
        return Err("内存测试正在进行中".into());
    }
    std::thread::spawn(move || {
        run_mem_test(app);
        MEM_TEST_RUN.store(false, Ordering::SeqCst);
    });
    Ok(())
}

fn run_mem_test(app: tauri::AppHandle) {
    use sysinfo::System;

    // 分配 min(可用内存 60%, 2GB)，按 64MB 分块
    let avail = System::new_all().available_memory();
    let total = ((avail as f64 * 0.6) as u64).min(2 * 1024 * 1024 * 1024) as usize;
    let block = total.min(64 * 1024 * 1024).max(1024 * 1024);
    let blocks = (total / block).max(1);

    let start = Instant::now();
    let mut errors: u64 = 0;
    let mut seed = 0xDEADBEEFu32;

    for i in 0..blocks {
        let mut buf: Vec<u8> = Vec::new();
        if buf.try_reserve_exact(block).is_err() {
            break; // 分配失败，结束并上报已有结果
        }
        buf.resize(block, 0);

        // 固定模式 0xAA / 0x55
        for pattern in [0xAAu8, 0x55u8] {
            buf.fill(pattern);
            errors += buf.iter().filter(|&&b| b != pattern).count() as u64;
        }

        // 随机模式写入 → 用同一序列回读校验
        let mut wseed = seed;
        for b in buf.iter_mut() {
            wseed = wseed.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = (wseed >> 24) as u8;
        }
        let mut rseed = seed;
        for &b in buf.iter() {
            rseed = rseed.wrapping_mul(1664525).wrapping_add(1013904223);
            if (rseed >> 24) as u8 != b {
                errors += 1;
            }
        }
        seed = seed.wrapping_add(0x9E3779B9);

        let _ = app.emit(
            "mem-test-progress",
            json!({ "pct": ((i + 1) * 100 / blocks), "errors": errors }),
        );
    }

    let _ = app.emit(
        "mem-test-done",
        json!({
            "errors": errors,
            "blocks_total": blocks,
            "seconds": start.elapsed().as_secs(),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::{vram_mb, DiskSpeed};

    #[test]
    fn vram_bytes_to_mb() {
        assert_eq!(vram_mb(8.0 * 1024.0 * 1024.0 * 1024.0), 8192);
        assert_eq!(vram_mb(512.0 * 1024.0 * 1024.0), 512);
    }

    #[test]
    fn vram_rejects_overflow_and_negative() {
        // WMI uint32 溢出时常见 -1 或超大值
        assert_eq!(vram_mb(-1.0), 0);
        assert_eq!(vram_mb(0.0), 0);
        assert_eq!(vram_mb(128.0 * 1024.0 * 1024.0 * 1024.0), 0);
    }

    #[test]
    fn disk_speed_json_keeps_snake_case() {
        // 前端读 read_mbps / write_mbps，不能被改成 readMbps
        let v = serde_json::to_value(DiskSpeed { read_mbps: 1.5, write_mbps: 2.5 }).unwrap();
        assert_eq!(v["read_mbps"], 1.5);
        assert_eq!(v["write_mbps"], 2.5);
    }
}
