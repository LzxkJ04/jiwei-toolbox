# 极维工具箱

Windows 一站式系统运维工具箱：批量软件安装、本机软件清单备份、Geek 式深度卸载、硬件检测与烤机、系统环境修复、软件备份。面向装机、二手验机、系统重装运维人员。

> 不内置任何激活/破解工具，许可证状态页仅只读查看系统激活状态。

## 下载

最新正式版：**v1.0.0** → [前往 Releases 下载](https://github.com/LzxkJ04/jiwei-toolbox/releases/latest)

| 文件 | 说明 | 大小 |
|---|---|---|
| `极维工具箱_Setup.exe` | 安装版，写入 `C:\Program Files\极维工具箱`，带开始菜单与桌面快捷方式、卸载程序 | 7.93 MB |
| `极维工具箱_便携单文件.exe` | 便携版，无需安装，双击即用；建议以 `--portable` 参数启动 | 20.73 MB |
| `SHA256_hash.txt` | 两个安装包的 SHA256 校验值 | — |

下载后请用 `Get-FileHash` 或 `certutil -hashfile` 核对 SHA256，与校验文件一致再进行安装。完整更新日志见 [CHANGELOG.md](CHANGELOG.md)。

> 便携版在多台机器间拷贝时，建议同时携带同目录的 `unattend.json`（如使用无人值守部署）。

## 技术栈

- **前端**：Vue 3 + TypeScript + Pinia + TailwindCSS + Radix Vue（shadcn-vue 风格）
- **后端**：Rust + Tauri 2（40+ 命令，53 个测试）
- **安装内核**：Inno Setup 静默逻辑包（内嵌进引导壳，全程无 Inno 原生窗口）

## 分发形态（单一 EXE，五种模式）

| 启动方式 | 模式 | 说明 |
|---|---|---|
| 双击（未安装） | **安装向导** | 自动检测未安装 → 7 页向导（协议/双路径/组件/依赖预检/进度/完成），后台 Inno 静默部署 |
| 双击（已安装） | **主程序** | 软件库/收藏/本机软件/任务队列/软件备份/硬件检测/系统工具/许可证状态/日志中心/设置 |
| `--installer` | 安装向导 | 强制进入 |
| `--uninstaller` | 卸载器 | 卸载确认 → 进度 → 完成（可选保留用户数据） |
| `--repair` | 修复模式 | 校验程序文件、重建目录配置、修复快捷方式与文件关联 |
| `--unattended` | 无人值守 | 读同目录 `unattend.json` 静默部署（企业批量分发），退出码 0/1/2 |
| `--portable` | 主程序（便携） | 跳过首装检测，直接运行 |

## 目录约定（双路径分离）

- **程序本体目录**：默认 `C:\Program Files\极维工具箱`——主程序与内置资源，安装覆盖、卸载删除
- **用户数据目录**：默认 `%LOCALAPPDATA%\极维工具箱`——配置/备份导出/缓存/日志，卸载默认保留
- 记录于 `dirs.json`（无 BOM UTF-8，五字段 program/data/config/backup/cache/log）

## 开发

```powershell
# 环境：Node 22（.workbuddy binaries）、Rust stable-msvc、Inno 7.1（C:\tools\inno）
npm install
npm run tauri:dev      # 开发运行
scripts\check.ps1      # 一键检查：cargo test + vue-tsc + vite + tauri build
npm run tauri:build    # 产出 src-tauri/target/release/jiwei-toolbox.exe
```

### 测试

```bash
cd src-tauri && cargo test            # 53 个测试（纯函数 + E2E 下载引擎）
cargo test probe -- --ignored        # 真实网络探针（需代理环境）
```

E2E 覆盖：本地 HTTP 服务器实测完整下载、HTTP Range 断点续传、令牌桶限速、取消中断。

## 构建产物

- `dist/jiwei-toolbox-portable.exe` — 单文件便携版（注意：`npm run build` 会清空 dist，需重新复制）
- `dist/unattend.json` — 无人值守部署模板（配合 `--unattended`）
- `installer/jiwei-logic.iss` → `dist-logic/jiwei-logic.exe` — Inno 静默逻辑包（编译后复制到 `src-tauri/resources/` 重新 tauri build 内嵌）

## 关键实现备忘（详见 .workbuddy/memory/）

- `mainBinaryName` 是 tauri.conf **顶层**字段；`bundle.active=false` 跳过打包
- Inno 7.1 **卸载阶段禁止调用 `WizardSilent()`**（用 `GetCmdTail` 解析 `/VERYSILENT`）
- `dirs.json` 写纯 ASCII `\uXXXX` 转义 JSON（无 BOM，任何解析器兼容）
- GUI 进程 reqwest 不读环境变量代理 → 图标下载三通道（系统代理/直连/本地端口兜底）
- hyper 发送小写 HTTP header，测试服务器匹配需大小写不敏感
