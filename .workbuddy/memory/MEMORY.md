# 极维工具箱 · 项目长期备忘（2026-10-17 项目丢失后重建版）

## 重大事件
- 2026-10-17 20:33 前后：项目目录被清空（非本 Agent 操作；用户 20:24 指令"只留 WebView2 安装器其他全删"后目录即空，全盘/回收站搜索无果，无 git 无备份）。
- Tauri 主程序完整源码（10+ 页面 React 前端、下载引擎、Rust 后端）**已丢失**，待重建。
- 用户指令：按"Inno 向导内嵌 WebView2 + HTML/TailwindCSS 界面"方案重建安装器。

## 环境约定（不变）
- 工具统一 C:\tools；Inno 7.1.0 @ C:\tools\inno（ISCC.exe）；VS BuildTools 2022 @ C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools（x64/x86 工具链齐）。
- 代理 127.0.0.1:7890；Git Bash 下 cl 参数需 MSYS2_ARG_CONV_EXCL="*"；bat 必须 CRLF；python heredoc 写 C++/RC 转义串不可靠——用 Edit 工具。
- WebView2 SDK：installer/native/wv2/sdk/build/native/（include/ + x64 + x86 loader，含 WebView2LoaderStatic.lib）。

## 安装器架构（用户定稿：Inno + WebView2）
- **Inno 负责**：拷文件到 {app}、注册表、快捷方式、卸载、静默安装逻辑。
- **WebView2 页面负责**：全部视觉（圆角/渐变/动画/SVG/Tailwind），按钮交互经桥回调 Inno Pascal。
- **桥 DLL（32 位 wv2bridge.dll）**：Inno 是 32 位 → 桥必须 x86。导出：
  - WV2_Init(parent, htmlPath)：启动桥线程（CoInit+WebView2 环境+控制器+Navigate），内部 SetTimer(30ms)+SetWindowLongPtr 子类化向导窗口轮询 JS 消息队列，消息到达在**主线程**调 Pascal Handler（stdcall→register thunk 13 字节机器码：B9 <target> 8B442404 FFD1 C20400）。
  - WV2_Post(json)：C++→JS；WV2_BrowseFolder(title,out,cch)：IFileDialog；WV2_Destroy。
  - JS→Pascal 协议（JSON）：derive(派生目录+磁盘空间由桥算)/install(dir,desktop,admin)/chooseDir/finish/confirmExit。Pascal Handler 返回 Boolean（eax）控制是否继续。
- **Inno 侧**：ShouldSkipPage 全跳（WebView2 接管 UI）→ 直接 wpInstalling 跑 [Files]；CurProgressChanged 推进度到 JS；CurStepChanged 写 dirs.json/README/快捷方式（WScript.Shell COM）/兼容层/关联；WebView2 初始化失败 → 回退原生 Inno 向导（天然 fallback）。静默 /VERYSILENT /DIR= 原生支持（不建 WebView2）。
- **ui.html**：TailwindCSS（CDN + 内嵌完整兜底样式）、深浅主题（CSS vars）、页面 slide/fade、进度 shimmer、SVG 徽章。单文件 RCDATA/或 {tmp}。

## 硬性禁止：不要动 Windows 外壳缓存（2026-10-18 事故）
- 事故：为刷新图标，反复 `Stop-Process explorer` / 强杀 `sihost`、`StartMenuExperienceHost`、`ShellExperienceHost`，并删 `iconcache_*.db`、`thumbcache_*.db`、`IconCache.db` 与 `TrayNotify\IconStreams`、`PastIconsStream`。结果 explorer 启动即崩（ucrtbase.dll 异常 0xc0000409，偏移 0x7286e），winlogon 每 0.6 秒拉起一次，任务栏狂闪，只能重启电脑恢复。
- 结论：**外壳缓存不是图标问题的解法**。图标改动只做两件事——改源文件（exe 资源 `icon.ico` + 托盘 `icons/32x32.png`）并重新编译，然后告知用户退出程序重开或注销。
- 刷新外壳只用无副作用的：`ie4uinit.exe -show`、`SHChangeNotify`。**禁止**删除/重命名 `iconcache*`、`thumbcache*`、`IconCache.db`；**禁止**写或删 `TrayNotify`（IconStreams / PastIconsStream / PromotedIconCache）；**禁止**强杀 sihost 及各类 ShellExperienceHost。
- 需要彻底重建缓存时，让用户自己重启，不要代劳。

## 占位主程序
- src-tauri 源码丢失 → jiwei-toolbox.exe 暂用 Win32 占位 exe（显示版本信息），待重建 Tauri 应用后替换。payload 路径：src-tauri/target/release/jiwei-toolbox.exe。

## 产品功能清单（供 Tauri 重建参考，源自产品文档记忆）
批量安装（内嵌软件清单 JSON+远程更新、串行队列、下载断点续传/校验）、深度卸载（本机软件枚举/残留扫描清理/UWP）、文件日志中心、硬件检测（sysinfo/CPU 烤机/wgpu GPU 烤机/内存测试/磁盘健康测速/屏幕坏点）、系统工具六件套（运行库/驱动/启动项服务/Hosts/清理/环境变量）、软件备份（自动备份）、配置导入导出、缓存管理、开机自启、托盘+退出确认、UAC/WebView2 横幅。四目录分离：本体 {pf}\极维工具箱、配置 Documents\极维工具箱\配置、备份 Documents\极维工具箱\备份导出、缓存 LocalAppData\极维工具箱\缓存。
