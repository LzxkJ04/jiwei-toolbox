//! 配置备份与恢复（.jwbak = zip 压缩包）

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

#[derive(Serialize)]
pub struct BackupInfo {
    pub file: String,
    pub size: u64,
    pub time: String,
}

#[derive(Serialize)]
pub struct ConfigProfile {
    pub id: String,
    pub name: String,
    pub paths: Vec<String>,
    pub exists: bool,
}

struct ManifestItem {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestoreStrategy {
    Skip,
    Overwrite,
    Rename,
}

enum ManifestInspect {
    Absent,
    Good { checked: u64 },
    Bad { checked: u64, mismatches: Vec<String> },
}

struct ProfileDef {
    id: &'static str,
    name: &'static str,
    /// (环境变量, 相对片段)。展开后路径不存在也要返回。
    paths: &'static [(&'static str, &'static [&'static str])],
}

const PROFILES: &[ProfileDef] = &[
    ProfileDef { id: "chrome", name: "Chrome", paths: &[("LOCALAPPDATA", &["Google", "Chrome", "User Data"])] },
    ProfileDef { id: "edge", name: "Edge", paths: &[("LOCALAPPDATA", &["Microsoft", "Edge", "User Data"])] },
    ProfileDef {
        id: "vscode",
        name: "VS Code",
        paths: &[("APPDATA", &["Code", "User"]), ("USERPROFILE", &[".vscode"])],
    },
    ProfileDef {
        id: "wechat",
        name: "微信",
        paths: &[("APPDATA", &["Tencent", "WeChat"]), ("APPDATA", &["Tencent", "xwechat"])],
    },
    ProfileDef {
        id: "qq",
        name: "QQ",
        paths: &[("APPDATA", &["Tencent", "QQ"]), ("APPDATA", &["Tencent", "QQNT"])],
    },
    ProfileDef {
        id: "netease-music",
        name: "网易云音乐",
        paths: &[
            ("LOCALAPPDATA", &["Netease", "CloudMusic"]),
            ("APPDATA", &["Netease", "CloudMusic"]),
        ],
    },
    ProfileDef {
        id: "potplayer",
        name: "PotPlayer",
        paths: &[("APPDATA", &["PotPlayerMini64"]), ("APPDATA", &["PotPlayerMini"])],
    },
    ProfileDef { id: "obs", name: "OBS", paths: &[("APPDATA", &["obs-studio"])] },
];

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn normalize_rel(path: &str) -> String {
    let mut p = path.replace('\\', "/");
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    p.trim_matches('/').to_string()
}

/// 由内存中的 (相对路径, 内容) 生成 manifest。路径统一为正斜杠，并按路径排序。
fn build_manifest(entries: &[(String, &[u8])]) -> serde_json::Value {
    let mut items: Vec<ManifestItem> = entries
        .iter()
        .map(|(path, bytes)| ManifestItem {
            path: normalize_rel(path),
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        })
        .collect();
    items.sort_by(|a, b| a.path.cmp(&b.path));
    manifest_value(&items)
}

fn manifest_value(items: &[ManifestItem]) -> serde_json::Value {
    let files: Vec<serde_json::Value> = items
        .iter()
        .map(|it| {
            serde_json::json!({
                "path": it.path,
                "size": it.size,
                "sha256": it.sha256,
            })
        })
        .collect();
    serde_json::json!({ "version": 1, "files": files })
}

fn manifest_files(value: &serde_json::Value) -> Result<Vec<ManifestItem>, String> {
    let arr = value
        .get("files")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "manifest.json 缺少 files 列表".to_string())?;
    let mut items = Vec::new();
    for (i, item) in arr.iter().enumerate() {
        let raw = item
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("manifest.json 第 {} 项缺少路径", i + 1))?;
        let path = normalize_rel(raw);
        if path.is_empty() || path.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
            return Err(format!("manifest.json 包含非法路径：{raw}"));
        }
        let size = item
            .get("size")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("manifest.json 中 {path} 缺少字节数"))?;
        let sha256 = item.get("sha256").and_then(|v| v.as_str()).unwrap_or("").trim().to_ascii_lowercase();
        if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("manifest.json 中 {path} 的 sha256 无效"));
        }
        items.push(ManifestItem { path, size, sha256 });
    }
    Ok(items)
}

fn diff_hashes(items: &[ManifestItem], actual: &HashMap<String, (u64, String)>) -> (u64, Vec<String>) {
    let mut mismatches = Vec::new();
    for it in items {
        match actual.get(&it.path) {
            None => mismatches.push(format!("文件 {} 在备份包中不存在", it.path)),
            Some((_size, sha)) if sha != &it.sha256 => {
                mismatches.push(format!("文件 {} 的 sha256 不匹配（清单 {}，实际 {}）", it.path, it.sha256, sha));
            }
            Some((size, _)) if *size != it.size => {
                mismatches.push(format!("文件 {} 的字节数不匹配（清单 {}，实际 {}）", it.path, it.size, size));
            }
            Some(_) => {}
        }
    }
    let checked = items.len() as u64;
    if mismatches.len() > 100 {
        let extra = mismatches.len() - 100;
        mismatches.truncate(100);
        mismatches.push(format!("其余 {extra} 处未列出"));
    }
    (checked, mismatches)
}

fn name_taken(existing_names: &HashSet<String>, name: &str) -> bool {
    existing_names.iter().any(|e| e.eq_ignore_ascii_case(name))
}

fn split_stem_ext(original: &str) -> (String, Option<String>) {
    let p = Path::new(original);
    let ext = p.extension().and_then(|e| e.to_str()).filter(|s| !s.is_empty()).map(|s| s.to_string());
    let stem = p.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or(original);
    (stem.to_string(), ext)
}

/// 目标目录已有同名文件时生成「原名 (2).ext」，序号递增直到不冲突。
fn unique_name(existing_names: &HashSet<String>, original: &str) -> String {
    let original = if original.is_empty() { "未命名" } else { original };
    if !name_taken(existing_names, original) {
        return original.to_string();
    }
    let (stem, ext) = split_stem_ext(original);
    let mut n = 2u32;
    loop {
        let candidate = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        if !name_taken(existing_names, &candidate) {
            return candidate;
        }
        n += 1;
        if n > 10_000 {
            return match &ext {
                Some(e) => format!("{stem} ({n}).{e}"),
                None => format!("{stem} ({n})"),
            };
        }
    }
}

fn parse_strategy(strategy: Option<String>) -> Result<RestoreStrategy, String> {
    let raw = strategy.unwrap_or_default();
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "overwrite" => Ok(RestoreStrategy::Overwrite),
        "skip" => Ok(RestoreStrategy::Skip),
        "rename" => Ok(RestoreStrategy::Rename),
        other => Err(format!("不支持的恢复策略：{other}（仅支持 skip、overwrite、rename）")),
    }
}

fn expand_env_path(var: &str, parts: &[&str]) -> String {
    match std::env::var(var) {
        Ok(base) if !base.trim().is_empty() => {
            let mut p = PathBuf::from(base.trim());
            for part in parts {
                p.push(part);
            }
            p.to_string_lossy().into_owned()
        }
        _ => {
            let mut s = format!("%{var}%");
            for part in parts {
                s.push('\\');
                s.push_str(part);
            }
            s
        }
    }
}

fn expand_profile_paths(def: &ProfileDef) -> Vec<String> {
    def.paths.iter().map(|(var, parts)| expand_env_path(var, parts)).collect()
}

fn profile_by_id(id: &str) -> Option<&'static ProfileDef> {
    PROFILES.iter().find(|p| p.id == id)
}

fn config_profiles() -> Vec<ConfigProfile> {
    PROFILES
        .iter()
        .map(|def| {
            let paths = expand_profile_paths(def);
            let exists = paths.iter().any(|p| Path::new(p).exists());
            ConfigProfile { id: def.id.to_string(), name: def.name.to_string(), paths, exists }
        })
        .collect()
}

fn join_zip_name(prefix: &str, rel: &str) -> String {
    let prefix = prefix.trim_matches('/');
    let rel = rel.trim_matches('/');
    if prefix.is_empty() {
        rel.to_string()
    } else if rel.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}/{rel}")
    }
}

fn allocate_prefix(id: &str, path: &Path, used: &mut HashSet<String>) -> String {
    let folder = path
        .file_name()
        .map(|s| s.to_string_lossy().replace(['/', '\\'], "_"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "dir".to_string());
    let mut candidate = format!("{id}/{folder}");
    let mut n = 2u32;
    while used.contains(&candidate) {
        candidate = format!("{id}/{folder}-{n}");
        n += 1;
        if n > 10_000 {
            break;
        }
    }
    used.insert(candidate.clone());
    candidate
}

/// file 必须位于 backup_dir 内、扩展名为 jwbak，拒绝 `..` 与盘符穿越。
fn ensure_inside_backup(backup_dir: &Path, file: &str) -> Result<PathBuf, String> {
    let file = file.trim();
    if file.is_empty() {
        return Err("请指定备份文件".into());
    }
    if file.chars().any(|c| c == '\0') {
        return Err("备份路径无效".into());
    }
    let raw = Path::new(file);
    if raw.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("备份路径不能包含 ..".into());
    }
    let mut seen_normal = false;
    for c in raw.components() {
        match c {
            Component::Prefix(_) if seen_normal => return Err("备份路径不能包含盘符穿越".into()),
            Component::Normal(_) => seen_normal = true,
            _ => {}
        }
    }
    if !raw.extension().map(|e| e.eq_ignore_ascii_case("jwbak")).unwrap_or(false) {
        return Err("只能校验扩展名为 .jwbak 的备份文件".into());
    }
    if !backup_dir.is_dir() {
        return Err("备份目录不存在".into());
    }
    let backup_canon = backup_dir.canonicalize().map_err(|e| format!("无法解析备份目录：{e}"))?;
    let candidate = if raw.is_absolute() { raw.to_path_buf() } else { backup_dir.join(raw) };
    if !candidate.is_file() {
        return Err("备份文件不存在，或位于备份目录之外".into());
    }
    let canon = candidate.canonicalize().map_err(|e| format!("无法解析备份文件：{e}"))?;
    if !canon.starts_with(&backup_canon) {
        return Err("拒绝访问备份目录之外的文件".into());
    }
    Ok(canon)
}

fn add_dir_to_zip(zw: &mut zip::ZipWriter<File>, dir: &Path, base: &Path) -> Result<(), String> {
    let mut sink = Vec::new();
    add_dir_recording(zw, dir, base, "", &mut sink, false)
}

/// 递归打包目录。`record` 为真时写入每个文件的路径、字节数和 sha256，并跳过符号链接。
fn add_dir_recording(
    zw: &mut zip::ZipWriter<File>,
    dir: &Path,
    base: &Path,
    zip_prefix: &str,
    manifest: &mut Vec<ManifestItem>,
    record: bool,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    for e in entries.flatten() {
        let p = e.path();
        let rel = p.strip_prefix(base).map_err(|e| e.to_string())?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let name = join_zip_name(zip_prefix, &rel_str);
        if record {
            let ft = e.file_type().map_err(|err| format!("{}: {}", p.display(), err))?;
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                add_dir_recording(zw, &p, base, zip_prefix, manifest, true)?;
            } else if ft.is_file() {
                let (size, sha) = write_hashed_file(zw, &name, &p)?;
                manifest.push(ManifestItem { path: normalize_rel(&name), size, sha256: sha });
            }
        } else if p.is_dir() {
            add_dir_recording(zw, &p, base, zip_prefix, manifest, false)?;
        } else if p.is_file() {
            zw.start_file(&name, zip::write::SimpleFileOptions::default())
                .map_err(|err| format!("{name}: {err}"))?;
            let mut f = File::open(&p).map_err(|err| format!("{}: {err}", p.display()))?;
            std::io::copy(&mut f, zw).map_err(|err| format!("{}: {err}", p.display()))?;
        }
    }
    Ok(())
}

fn write_hashed_file(zw: &mut zip::ZipWriter<File>, name: &str, path: &Path) -> Result<(u64, String), String> {
    let mut f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    zw.start_file(name, zip::write::SimpleFileOptions::default())
        .map_err(|e| format!("{name}: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        zw.write_all(&buf[..n]).map_err(|e| format!("{name}: {e}"))?;
        size += n as u64;
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

fn write_json_entry(zw: &mut zip::ZipWriter<File>, name: &str, value: &serde_json::Value) -> Result<(), String> {
    zw.start_file(name, zip::write::SimpleFileOptions::default())
        .map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    zw.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

fn write_manifest(zw: &mut zip::ZipWriter<File>, items: &mut Vec<ManifestItem>) -> Result<(), String> {
    items.sort_by(|a, b| a.path.cmp(&b.path));
    write_json_entry(zw, "manifest.json", &manifest_value(items))
}

fn remove_partial(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// 创建配置备份：打包 config 目录 → backup\backup-YYYYMMDD-HHMMSS.jwbak
#[tauri::command]
pub async fn create_backup(note: String) -> Result<serde_json::Value, String> {
    let dirs = crate::dirs::load_dirs();
    let src = PathBuf::from(&dirs.config);
    if !src.is_dir() {
        return Err(format!("配置目录不存在: {}", dirs.config));
    }
    let backup_dir = PathBuf::from(&dirs.backup);
    std::fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;

    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = backup_dir.join(format!("backup-{}.jwbak", ts));
    write_config_archive(&dest, &src, &note)?;
    crate::logs::op_log(&dirs, "backup", "create_backup", &format!("创建备份 {}（备注: {}）", dest.to_string_lossy(), note));
    Ok(serde_json::json!({ "file": dest.to_string_lossy() }))
}

fn write_config_archive(dest: &Path, src: &Path, note: &str) -> Result<(), String> {
    let file = match File::create(dest) {
        Ok(f) => f,
        Err(e) => return Err(format!("无法创建备份文件：{e}")),
    };
    let write = (|| -> Result<(), String> {
        let mut zw = zip::ZipWriter::new(file);
        write_json_entry(
            &mut zw,
            "_meta.json",
            &serde_json::json!({
                "note": note,
                "time": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                "kind": "config",
            }),
        )?;
        let mut manifest = Vec::new();
        add_dir_recording(&mut zw, src, src, "", &mut manifest, true)?;
        // 控制清单放在包尾，避免和用户自己的 manifest.json 抢第一条记录
        write_manifest(&mut zw, &mut manifest)?;
        zw.finish().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if write.is_err() {
        remove_partial(dest);
    }
    write
}

/// 列出全部备份
#[tauri::command]
pub async fn list_backups() -> Vec<BackupInfo> {
    let dirs = crate::dirs::load_dirs();
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dirs.backup) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.extension().map(|x| x.eq_ignore_ascii_case("jwbak")).unwrap_or(false) {
                continue;
            }
            if let Ok(meta) = e.metadata() {
                let time = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
                    .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_default();
                out.push(BackupInfo { file: p.to_string_lossy().into_owned(), size: meta.len(), time });
            }
        }
    }
    out.sort_by(|a, b| b.file.cmp(&a.file));
    out
}

/// 恢复备份。strategy 缺省 overwrite，与旧前端只传 file 时一致。
/// 包内若有 manifest.json，先校验 sha256，失败则直接拒绝，不改用户文件。
#[tauri::command]
pub fn restore_backup(file: String, strategy: Option<String>) -> Result<serde_json::Value, String> {
    let strategy = parse_strategy(strategy)?;
    let dirs = crate::dirs::load_dirs();
    let zf = File::open(&file).map_err(|e| format!("{file}: {e}"))?;
    let mut archive = zip::ZipArchive::new(zf).map_err(|e| e.to_string())?;

    match inspect_manifest(&mut archive)? {
        ManifestInspect::Bad { mismatches, .. } => return Err(format_restore_reject(&mismatches)),
        ManifestInspect::Absent | ManifestInspect::Good { .. } => {}
    }

    let indexes = list_file_indexes(&mut archive)?;
    // _meta.json 写在包头，只跳过第一份（我们的备注）；用户自己的同名文件仍恢复。
    // manifest.json 写在包尾，只跳过最后一份（控制清单）。
    let meta_idx = indexes.iter().find(|(_, name)| name == "_meta.json").map(|(idx, _)| *idx);
    let control_idx = indexes.iter().rev().find(|(_, name)| name == "manifest.json").map(|(idx, _)| *idx);

    let cfg = PathBuf::from(&dirs.config);
    std::fs::create_dir_all(&cfg).map_err(|e| e.to_string())?;
    let cfg_canon = cfg.canonicalize().map_err(|e| format!("无法解析配置目录：{e}"))?;

    let mut written = 0u64;
    let mut skipped = 0u64;
    let mut renamed = 0u64;
    let mut reserved: HashMap<PathBuf, HashSet<String>> = HashMap::new();

    for i in 0..archive.len() {
        if Some(i) == control_idx || Some(i) == meta_idx {
            continue;
        }
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let mut dest = cfg.join(&rel);
        if !dest.starts_with(&cfg) {
            return Err(format!("拒绝恢复非法路径：{}", rel.display()));
        }
        if entry.name().ends_with('/') {
            ensure_restore_parent(&cfg, &cfg_canon, &dest)?;
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            continue;
        }
        if strategy == RestoreStrategy::Skip && dest.exists() {
            skipped += 1;
            continue;
        }
        if strategy == RestoreStrategy::Rename && dest.exists() {
            let parent = dest.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| cfg.clone());
            let orig = dest
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "未命名".to_string());
            let names = reserved.entry(parent.clone()).or_insert_with(|| dir_names(&parent));
            let new_name = unique_name(names, &orig);
            names.insert(new_name.clone());
            dest = parent.join(new_name);
            renamed += 1;
        } else {
            written += 1;
        }
        if let Some(parent) = dest.parent() {
            ensure_restore_parent(&cfg, &cfg_canon, parent)?;
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = File::create(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("{}: {e}", dest.display()))?;
    }

    let strategy_name = match strategy {
        RestoreStrategy::Skip => "skip",
        RestoreStrategy::Overwrite => "overwrite",
        RestoreStrategy::Rename => "rename",
    };
    crate::logs::op_log(
        &dirs,
        "backup",
        "restore_backup",
        &format!("恢复备份 {file} 策略 {strategy_name} 写入 {written} 跳过 {skipped} 重命名 {renamed}"),
    );
    Ok(serde_json::json!({
        "strategy": strategy_name,
        "written": written,
        "skipped": skipped,
        "renamed": renamed,
    }))
}

/// 先确认最近的已存在父目录仍在配置目录内，再创建子目录，避免目录联接把文件写出配置目录。
fn ensure_restore_parent(cfg: &Path, cfg_canon: &Path, parent: &Path) -> Result<(), String> {
    let mut cursor = parent.to_path_buf();
    loop {
        if cursor.exists() {
            let canon = cursor.canonicalize().map_err(|e| format!("无法解析目录：{e}"))?;
            if canon != cfg_canon && !canon.starts_with(cfg_canon) {
                return Err("拒绝将文件恢复到配置目录之外".into());
            }
            return Ok(());
        }
        if cursor == cfg {
            return Ok(());
        }
        match cursor.parent() {
            Some(next) if next != cursor.as_path() => cursor = next.to_path_buf(),
            _ => return Ok(()),
        }
    }
}

fn dir_names(dir: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            set.insert(e.file_name().to_string_lossy().into_owned());
        }
    }
    set
}

fn format_restore_reject(mismatches: &[String]) -> String {
    let show = mismatches.len().min(8);
    let mut msg = format!(
        "备份校验失败，已取消恢复，未修改任何文件：{}",
        mismatches.iter().take(show).cloned().collect::<Vec<_>>().join("；")
    );
    if mismatches.len() > show {
        msg.push_str(&format!("；另有 {} 处不一致", mismatches.len() - show));
    }
    msg
}

fn list_file_indexes(archive: &mut zip::ZipArchive<File>) -> Result<Vec<(usize, String)>, String> {
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| format!("读取备份条目失败：{e}"))?;
        let is_dir = entry.name().ends_with('/');
        let name = normalize_rel(entry.name());
        if is_dir || name.is_empty() {
            continue;
        }
        out.push((i, name));
    }
    Ok(out)
}

fn read_entry_bytes(archive: &mut zip::ZipArchive<File>, index: usize) -> Result<Vec<u8>, String> {
    let mut entry = archive.by_index(index).map_err(|e| format!("读取备份条目失败：{e}"))?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).map_err(|e| format!("读取备份条目失败：{e}"))?;
    Ok(buf)
}

fn hash_zip_entry(archive: &mut zip::ZipArchive<File>, index: usize) -> Result<(u64, String), String> {
    let mut entry = archive.by_index(index).map_err(|e| format!("读取备份条目失败：{e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = entry.read(&mut buf).map_err(|e| format!("读取备份条目失败：{e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

/// 同名条目有多份时（例如用户自己的 _meta.json 和我们写的备注），选中哈希相符的那一份。
fn choose_payload_index(
    archive: &mut zip::ZipArchive<File>,
    indexes: &[(usize, String)],
    control_idx: usize,
    item: &ManifestItem,
) -> Result<Option<usize>, String> {
    let candidates: Vec<usize> = indexes
        .iter()
        .filter(|(idx, name)| name == &item.path && *idx != control_idx)
        .map(|(idx, _)| *idx)
        .collect();
    if candidates.len() <= 1 {
        return Ok(candidates.first().copied());
    }
    for idx in &candidates {
        let (size, sha) = hash_zip_entry(archive, *idx)?;
        if size == item.size && sha == item.sha256 {
            return Ok(Some(*idx));
        }
    }
    Ok(candidates.first().copied())
}

/// 只读检查包内 manifest。没有 manifest 时返回 Absent（旧备份仍可恢复）。
fn inspect_manifest(archive: &mut zip::ZipArchive<File>) -> Result<ManifestInspect, String> {
    let indexes = list_file_indexes(archive)?;
    let Some(control_idx) = indexes.iter().rev().find(|(_, name)| name == "manifest.json").map(|(idx, _)| *idx) else {
        return Ok(ManifestInspect::Absent);
    };
    let bytes = read_entry_bytes(archive, control_idx)?;
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            return Ok(ManifestInspect::Bad {
                checked: 0,
                mismatches: vec!["manifest.json 不是有效的 JSON".into()],
            });
        }
    };
    let items = match manifest_files(&value) {
        Ok(v) => v,
        Err(e) => return Ok(ManifestInspect::Bad { checked: 0, mismatches: vec![e] }),
    };
    let mut actual = HashMap::new();
    for item in &items {
        let Some(idx) = choose_payload_index(archive, &indexes, control_idx, item)? else { continue };
        let hashed = hash_zip_entry(archive, idx)?;
        actual.insert(item.path.clone(), hashed);
    }
    let (checked, mismatches) = diff_hashes(&items, &actual);
    if mismatches.is_empty() {
        Ok(ManifestInspect::Good { checked })
    } else {
        Ok(ManifestInspect::Bad { checked, mismatches })
    }
}

/// 列出内置软件配置目录。路径由 APPDATA / LOCALAPPDATA / USERPROFILE 展开，不存在也返回。
#[tauri::command]
pub async fn list_config_profiles() -> Vec<ConfigProfile> {
    config_profiles()
}

/// 把选中且实际存在的配置目录打成 config-时间.jwbak。未知 id 拒绝；没有可备份目录时返回中文错误。
#[tauri::command]
pub async fn backup_profiles(ids: Vec<String>, note: String) -> Result<serde_json::Value, String> {
    if ids.is_empty() {
        return Err("请至少选择一个软件配置".into());
    }
    let mut ordered: Vec<&'static ProfileDef> = Vec::new();
    let mut seen = HashSet::new();
    for id in &ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        if !seen.insert(id.to_string()) {
            continue;
        }
        match profile_by_id(id) {
            Some(def) => ordered.push(def),
            None => return Err(format!("未知的配置档案：{id}")),
        }
    }
    if ordered.is_empty() {
        return Err("请至少选择一个软件配置".into());
    }

    let mut plan: Vec<(&'static ProfileDef, Vec<String>)> = Vec::new();
    for def in &ordered {
        let existing: Vec<String> = expand_profile_paths(def).into_iter().filter(|p| Path::new(p).is_dir()).collect();
        if !existing.is_empty() {
            plan.push((*def, existing));
        }
    }
    if plan.is_empty() {
        return Err("没有可备份的配置目录（所选软件的配置路径均不存在）".into());
    }

    let dirs = crate::dirs::load_dirs();
    let backup_dir = PathBuf::from(&dirs.backup);
    std::fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut dest = backup_dir.join(format!("config-{ts}.jwbak"));
    let mut seq = 2u32;
    while dest.exists() {
        dest = backup_dir.join(format!("config-{ts}-{seq}.jwbak"));
        seq += 1;
    }
    write_profiles_archive(&dest, &note, &plan)?;
    let saved: Vec<&str> = plan.iter().map(|(def, _)| def.id).collect();
    crate::logs::op_log(
        &dirs,
        "backup",
        "backup_profiles",
        &format!("打包软件配置 {} -> {}", saved.join(","), dest.to_string_lossy()),
    );
    Ok(serde_json::json!({ "file": dest.to_string_lossy() }))
}

fn write_profiles_archive(dest: &Path, note: &str, plan: &[(&'static ProfileDef, Vec<String>)]) -> Result<(), String> {
    let file = match File::create(dest) {
        Ok(f) => f,
        Err(e) => return Err(format!("无法创建备份文件：{e}")),
    };
    let write = (|| -> Result<(), String> {
        let mut zw = zip::ZipWriter::new(file);
        let ids: Vec<&str> = plan.iter().map(|(def, _)| def.id).collect();
        write_json_entry(
            &mut zw,
            "_meta.json",
            &serde_json::json!({
                "note": note,
                "time": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                "kind": "profiles",
                "ids": ids,
            }),
        )?;
        let mut manifest = Vec::new();
        let mut used = HashSet::new();
        for (def, paths) in plan {
            for path in paths {
                let pb = Path::new(path);
                let prefix = allocate_prefix(def.id, pb, &mut used);
                add_dir_recording(&mut zw, pb, pb, &prefix, &mut manifest, true)?;
            }
        }
        write_manifest(&mut zw, &mut manifest)?;
        zw.finish().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if write.is_err() {
        remove_partial(dest);
    }
    write
}

/// 只读校验备份目录内的 .jwbak。返回 {ok, checked, mismatches}。
#[tauri::command]
pub async fn verify_backup(file: String) -> Result<serde_json::Value, String> {
    let dirs = crate::dirs::load_dirs();
    let path = ensure_inside_backup(Path::new(&dirs.backup), &file)?;
    let zf = File::open(&path).map_err(|e| format!("无法打开备份：{e}"))?;
    let mut archive = zip::ZipArchive::new(zf).map_err(|e| format!("无法读取备份包：{e}"))?;
    let (ok, checked, mismatches) = match inspect_manifest(&mut archive)? {
        ManifestInspect::Absent => (false, 0u64, vec!["备份包内没有 manifest.json，无法校验".to_string()]),
        ManifestInspect::Good { checked } => (true, checked, Vec::new()),
        ManifestInspect::Bad { checked, mismatches } => (false, checked, mismatches),
    };
    crate::logs::op_log(
        &dirs,
        "backup",
        "verify_backup",
        &format!("校验 {} ok={ok} checked={checked}", path.display()),
    );
    Ok(serde_json::json!({ "ok": ok, "checked": checked, "mismatches": mismatches }))
}

/// 递归复制目录（用于导入前备份现有配置）
fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {}", dest.display(), e))?;
    for e in std::fs::read_dir(src).map_err(|e| format!("{}: {}", src.display(), e))?.flatten() {
        let p = e.path();
        let target = dest.join(p.file_name().unwrap_or_default());
        if p.is_dir() {
            copy_dir(&p, &target)?;
        } else if p.is_file() {
            std::fs::copy(&p, &target).map_err(|e| format!("{}: {}", p.display(), e))?;
        }
    }
    Ok(())
}

/// 导出配置：把配置目录打包 zip（config-export-YYYYMMDD-HHMMSS.zip）到备份导出目录；
/// target 可指定导出目标目录（缺省用备份目录）。返回 {file}
#[tauri::command]
pub fn export_config(target: Option<String>) -> Result<serde_json::Value, String> {
    let dirs = crate::dirs::load_dirs();
    let src = PathBuf::from(&dirs.config);
    if !src.is_dir() {
        return Err(format!("配置目录不存在: {}", dirs.config));
    }
    let dest_dir = match target.as_deref() {
        Some(t) if !t.trim().is_empty() => PathBuf::from(t),
        _ => PathBuf::from(&dirs.backup),
    };
    std::fs::create_dir_all(&dest_dir).map_err(|e| format!("{}: {}", dest_dir.display(), e))?;

    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = dest_dir.join(format!("config-export-{}.zip", ts));
    let file = File::create(&dest).map_err(|e| e.to_string())?;
    let mut zw = zip::ZipWriter::new(file);
    add_dir_to_zip(&mut zw, &src, &src)?;
    zw.finish().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "file": dest.to_string_lossy() }))
}

/// 导入配置：解压 zip 覆盖配置目录；覆盖前先把现有配置备份到
/// 备份目录下 config-import-bak-YYYYMMDD-HHMMSS
#[tauri::command]
pub fn import_config(file: String) -> Result<(), String> {
    let dirs = crate::dirs::load_dirs();
    let zf = File::open(&file).map_err(|e| format!("{}: {}", file, e))?;
    let mut archive = zip::ZipArchive::new(zf).map_err(|e| e.to_string())?;
    let cfg = PathBuf::from(&dirs.config);
    std::fs::create_dir_all(&cfg).map_err(|e| e.to_string())?;

    // 覆盖前备份现有配置
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let bak = PathBuf::from(&dirs.backup).join(format!("config-import-bak-{}", ts));
    copy_dir(&cfg, &bak).map_err(|e| format!("备份现有配置失败: {}", e))?;

    // 解压覆盖（含路径穿越防护）
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else { continue };
        if entry.name().ends_with('/') {
            std::fs::create_dir_all(cfg.join(&rel)).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = cfg.join(&rel).parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = File::create(cfg.join(&rel)).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 开机自动备份软件清单（settings.json auto_backup_inventory: bool）
// ---------------------------------------------------------------------------

/// 读取"开机自动备份清单"开关（默认关闭）
#[tauri::command]
pub fn get_auto_backup_inventory() -> bool {
    crate::common::read_setting_raw("auto_backup_inventory")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 设置"开机自动备份清单"开关
#[tauri::command]
pub fn set_auto_backup_inventory(enable: bool) -> Result<(), String> {
    crate::common::write_setting_raw("auto_backup_inventory", serde_json::Value::Bool(enable))
}

/// 枚举已安装软件并写入 <data>\备份导出\software-inventory-auto-YYYYMMDD.json
/// （UTF-8 JSON 数组：[{name, version, publisher, install_location}]）+ op_log
fn write_inventory_snapshot(dirs: &crate::dirs::Dirs) -> Result<String, String> {
    let apps = crate::uninstall::list_installed_inner();
    let items: Vec<serde_json::Value> = apps
        .iter()
        .map(|a| {
            serde_json::json!({
                "name": a.name,
                "version": a.version,
                "publisher": a.publisher,
                "install_location": a.install_location,
            })
        })
        .collect();
    let backup_dir = PathBuf::from(&dirs.backup);
    std::fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;
    let dest = backup_dir.join(format!(
        "software-inventory-auto-{}.json",
        chrono::Local::now().format("%Y%m%d")
    ));
    let text = serde_json::to_string_pretty(&items).map_err(|e| e.to_string())?;
    std::fs::write(&dest, text).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().into_owned())
}

/// 启动时后台自动备份软件清单（lib.rs setup 主模式下调用）：
/// 开关开启时延迟 5 秒等待启动高峰过后写清单快照
pub fn startup_auto_inventory() {
    std::thread::spawn(|| {
        if !get_auto_backup_inventory() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
        let dirs = crate::dirs::load_dirs();
        match write_inventory_snapshot(&dirs) {
            Ok(path) => crate::logs::op_log(&dirs, "backup", "auto_inventory", &format!("开机自动备份软件清单 -> {}", path)),
            Err(e) => crate::logs::op_log(&dirs, "backup", "auto_inventory", &format!("开机自动备份软件清单失败: {}", e)),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO_SHA: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn build_manifest_records_relative_path_size_and_sha256() {
        let manifest = build_manifest(&[("a.txt".into(), b"hello".as_slice())]);
        let files = manifest["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["path"], "a.txt");
        assert_eq!(files[0]["size"], 5);
        assert_eq!(files[0]["sha256"], HELLO_SHA);
        assert_eq!(manifest["version"], 1);
    }

    #[test]
    fn build_manifest_normalizes_and_sorts_paths() {
        let manifest = build_manifest(&[
            ("z\\b.txt".into(), b"world".as_slice()),
            ("./a.txt".into(), b"hello".as_slice()),
        ]);
        let files = manifest["files"].as_array().unwrap();
        assert_eq!(files[0]["path"], "a.txt");
        assert_eq!(files[1]["path"], "z/b.txt");
        assert_eq!(files[1]["size"], 5);
    }

    #[test]
    fn diff_hashes_reports_mismatch_and_missing() {
        let manifest = build_manifest(&[
            ("a.txt".into(), b"hello".as_slice()),
            ("b.txt".into(), b"world".as_slice()),
        ]);
        let items = manifest_files(&manifest).unwrap();
        let mut actual = HashMap::new();
        actual.insert("a.txt".into(), (5u64, HELLO_SHA.to_string()));
        let (checked, bad) = diff_hashes(&items, &actual);
        assert_eq!(checked, 2);
        assert!(bad.iter().any(|m| m.contains("b.txt") && m.contains("不存在")));
        assert!(!bad.iter().any(|m| m.contains("a.txt")));

        actual.insert("b.txt".into(), (5u64, "ab".repeat(32)));
        let (_, bad) = diff_hashes(&items, &actual);
        assert!(bad.iter().any(|m| m.contains("b.txt") && m.contains("sha256")));
    }

    #[test]
    fn unique_name_keeps_free_name() {
        let existing = HashSet::new();
        assert_eq!(unique_name(&existing, "file.txt"), "file.txt");
        assert_eq!(unique_name(&existing, "readme"), "readme");
    }

    #[test]
    fn unique_name_inserts_index_before_extension() {
        let existing = HashSet::from(["file.txt".to_string()]);
        assert_eq!(unique_name(&existing, "file.txt"), "file (2).txt");
        let dot = HashSet::from([".gitignore".to_string()]);
        assert_eq!(unique_name(&dot, ".gitignore"), ".gitignore (2)");
        let bare = HashSet::from(["notes".to_string()]);
        assert_eq!(unique_name(&bare, "notes"), "notes (2)");
    }

    #[test]
    fn unique_name_skips_existing_indexes() {
        let existing = HashSet::from(["file.txt".to_string(), "file (2).txt".to_string()]);
        assert_eq!(unique_name(&existing, "file.txt"), "file (3).txt");
    }

    #[test]
    fn unique_name_is_case_insensitive() {
        let existing = HashSet::from(["File.TXT".to_string(), "FILE (2).txt".to_string()]);
        assert_eq!(unique_name(&existing, "file.txt"), "file (3).txt");
    }

    #[test]
    fn parse_strategy_defaults_to_overwrite() {
        assert_eq!(parse_strategy(None).unwrap(), RestoreStrategy::Overwrite);
        assert_eq!(parse_strategy(Some("  ".into())).unwrap(), RestoreStrategy::Overwrite);
        assert_eq!(parse_strategy(Some("Skip".into())).unwrap(), RestoreStrategy::Skip);
        assert_eq!(parse_strategy(Some("rename".into())).unwrap(), RestoreStrategy::Rename);
        assert!(parse_strategy(Some("merge".into())).unwrap_err().contains("不支持"));
    }

    #[test]
    fn builtin_profiles_include_required_software() {
        let ids: Vec<&str> = PROFILES.iter().map(|p| p.id).collect();
        for id in ["chrome", "edge", "vscode", "wechat", "qq", "netease-music", "potplayer", "obs"] {
            assert!(ids.contains(&id), "{id}");
        }
        assert!(PROFILES.len() >= 8);
        assert!(profile_by_id("no-such-app").is_none());
        assert!(PROFILES.iter().any(|p| p.paths.iter().any(|pair| pair.0 == "USERPROFILE")));
        assert!(PROFILES.iter().any(|p| p.paths.iter().any(|pair| pair.0 == "APPDATA")));
        assert!(PROFILES.iter().any(|p| p.paths.iter().any(|pair| pair.0 == "LOCALAPPDATA")));
    }

    #[test]
    fn ensure_inside_backup_rejects_parent_and_outside() {
        let tmp = std::env::temp_dir().join(format!(
            "jw-backup-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
        ));
        let backup = tmp.join("备份导出");
        std::fs::create_dir_all(&backup).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(tmp.clone());

        let inside = backup.join("ok.jwbak");
        std::fs::write(&inside, b"jw").unwrap();
        let outside = tmp.join("outside.jwbak");
        std::fs::write(&outside, b"no").unwrap();
        let txt = backup.join("note.txt");
        std::fs::write(&txt, b"x").unwrap();

        assert!(ensure_inside_backup(&backup, inside.to_str().unwrap()).is_ok());
        assert!(ensure_inside_backup(&backup, "ok.jwbak").is_ok());
        assert!(ensure_inside_backup(&backup, outside.to_str().unwrap()).is_err());
        assert!(ensure_inside_backup(&backup, txt.to_str().unwrap()).unwrap_err().contains("jwbak"));
        assert!(ensure_inside_backup(&backup, r"..\outside.jwbak").unwrap_err().contains(".."));
        let nested = format!("{}\\..\\outside.jwbak", backup.display());
        assert!(ensure_inside_backup(&backup, &nested).is_err());
        assert!(ensure_inside_backup(&backup, r"sub\C:\secret.jwbak").is_err());
    }
}
