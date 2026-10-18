# 发布流程

面向维护者的操作手册。目标是让每次发版可重复、可核查，不依赖记忆。

## 一、发布前检查

### 1. 版本号统一

版本号唯一来源是 `package.json` 的 `version`。以下四处必须同步修改，否则 exe 属性、安装包属性与页面显示会不一致：

| 位置 | 字段 | 说明 |
|---|---|---|
| `package.json` | `version` | 前端构建期注入 `__APP_VERSION__`，界面显示取此值 |
| `src-tauri/tauri.conf.json` | 顶层 `version` | 决定 exe 文件属性版本 |
| `src-tauri/Cargo.toml` | `[package] version` | Windows 资源版本信息来源 |
| `installer/极维工具箱.iss` | `#define MyAppVersion` | 安装包 `AppVersion` 与 `VersionInfoVersion` |

界面代码中不再硬编码版本号：`AboutView.vue` 与 `InstallerView.vue` 均使用构建期注入的 `__APP_VERSION__`。

快速核对：

```powershell
Select-String -Path package.json,src-tauri\tauri.conf.json,src-tauri\Cargo.toml,installer\极维工具箱.iss -Pattern '1\.0\.0'
```

### 2. 完整构建

```powershell
# 前端（会清空 dist）
npm run build

# 主程序（需 MSVC 环境变量，见 README）
cd src-tauri; cargo build --release

# 便携版：前端构建会清空 dist，必须重新复制
Copy-Item src-tauri\target\release\jiwei-toolbox.exe dist\jiwei-toolbox-portable.exe -Force

# 安装包
C:\tools\inno\ISCC.exe "installer\极维工具箱.iss"
```

构建完成后核对 exe 属性：

```powershell
(Get-Item dist\jiwei-setup.exe).VersionInfo.FileVersion
(Get-Item src-tauri\target\release\jiwei-toolbox.exe).VersionInfo.FileVersion
```

两者都应等于目标版本号。

### 3. 冒烟测试

普通权限与管理员权限各跑一遍，重点覆盖：

- 软件库加载、搜索、加入队列、依赖检查
- 深度卸载流程、残留扫描、占用文件的重启删除登记
- 备份导出与还原（含错误校验路径）
- 硬件检测各栏与 CPU 烤机超温终止
- 深浅主题切换、配置导入导出、任务模板加载
- 安装向导 7 页流程、卸载器、`--repair`、`--unattended`、`--portable`
- 关于页版本号是否与目标版本一致

### 4. 产物整理

```powershell
# 按发布命名整理附件
New-Item -ItemType Directory release -Force
Copy-Item dist\jiwei-toolbox-portable.exe "release\极维工具箱_便携单文件.exe" -Force
Copy-Item dist\jiwei-setup.exe          "release\极维工具箱_Setup.exe" -Force

# 计算校验值
Get-FileHash "release\极维工具箱_便携单文件.exe" -Algorithm SHA256
Get-FileHash "release\极维工具箱_Setup.exe" -Algorithm SHA256
```

把结果写入 `release\SHA256_hash.txt`，格式见该文件现有内容。

## 二、提交与打标签

```powershell
git add -A
git commit -m "发布 v1.0.0：首个正式版"
git tag -a v1.0.0 -m "极维工具箱 v1.0.0"
git push origin main
git push origin v1.0.0
```

标签规范：`v主版本.次版本.修订号`，预发布加后缀，如 `v1.0.0-beta1`。

## 三、创建 GitHub Release

1. 仓库页右侧 **Releases** → **Create a new release**
2. 选择刚推送的标签
3. 标题：`极维工具箱 v1.0.0`
4. 正式版不勾选 pre-release；Beta 包必须勾选
5. 正文使用下方模板
6. 上传附件：`极维工具箱_便携单文件.exe`、`极维工具箱_Setup.exe`、`SHA256_hash.txt`
7. 核对文件名无乱码、体积正确后 **Publish release**

### Release 正文模板

```markdown
# 极维工具箱 vX.X.X
Windows 系统运维工具箱 | Tauri 2 + Vue 3

支持批量软件安装、深度卸载、软件备份还原、硬件检测、系统工具，面向装机与二手验机场景。

## 新增功能
-

## UI 优化
-

## 修复
-

## 下载
| 文件 | 说明 |
|---|---|
| 极维工具箱_Setup.exe | 安装版 |
| 极维工具箱_便携单文件.exe | 便携版，建议加 --portable 启动 |
| SHA256_hash.txt | 校验值 |

## 使用须知
1. 推荐以管理员权限运行，解锁系统级功能
2. 深度卸载与注册表操作存在风险，请谨慎操作并提前备份
3. 加密备份包密码遗失无法恢复
4. 本工具仅作系统辅助用途，操作风险由使用者自行承担

## 反馈
缺陷请提交 Issues，使用问题与建议请到 Discussions。
```

## 四、发布后操作

1. 从 Release 页实际下载一次，确认文件完整、可正常启动
2. 更新 `README.md` 的下载区域，指向最新 Release
3. 在 Discussions 发布版本公告
4. 若内置更新模块依赖更新服务配置，同步写入新版本号与下载地址
5. 在 `docs/releases/vX.X.X.md` 归档本次日志快照

## 五、版本迭代规范

- 主版本：架构级改动或大批新模块
- 次版本：新增功能页面或工具模块
- 修订版本：缺陷修复、UI 微调、文案修改

分支策略：

| 分支 | 用途 |
|---|---|
| `main` | 每个正式版本的代码，保持稳定 |
| `dev` | 日常开发与功能合并 |
| `feature/*` | 单个模块独立开发，完成后并入 `dev` |
| `hotfix/*` | `main` 上紧急修复，修完打修订标签发布 |

## 六、风险检查清单

- [ ] 四处版本号完全一致
- [ ] 安装包与便携版均可正常启动
- [ ] 关于页与 exe 属性显示的版本正确
- [ ] 更新日志内容与实际改动一致
- [ ] SHA256 校验文件与实际产物一致
- [ ] Release 文案包含完整免责声明
- [ ] 未把本地配置、密钥、日志、临时文件打包进产物
- [ ] 附件文件名无乱码、体积与本地一致
