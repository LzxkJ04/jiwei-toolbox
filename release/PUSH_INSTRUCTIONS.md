# 推送到 GitHub

仓库当前**没有配置远程**。拿到仓库地址后，按下面步骤执行（只需改第一行）。

## 1. 配置远程

```powershell
# 把 OWNER/REPO 换成你的实际仓库
git remote add origin https://github.com/OWNER/REPO.git

# 验证
git remote -v
```

## 2. 推送代码与标签

```powershell
git push -u origin main
git push origin v1.0.0
```

若使用 SSH：

```powershell
git remote add origin git@github.com:OWNER/REPO.git
```

## 3. 替换 Issue 模板中的占位地址

`.github/ISSUE_TEMPLATE/config.yml` 里有两处 `OWNER/REPO`，替换成实际地址后提交：

```powershell
# 用实际地址替换（示例）
(Get-Content .github\ISSUE_TEMPLATE\config.yml) -replace 'OWNER/REPO', 'yourname/jiwei-toolbox' |
  Set-Content .github\ISSUE_TEMPLATE\config.yml -Encoding UTF8

git add .github/ISSUE_TEMPLATE/config.yml
git commit -m "补全 Issue 模板中的仓库地址"
git push
```

## 4. 创建 Release

仓库页 → Releases → Create a new release → 选择 `v1.0.0` 标签。

- 标题：`极维工具箱 v1.0.0`
- 正文：直接粘贴 `release/Release_Notes_v1.0.0.md` 的内容
- 正式版不要勾选 pre-release
- 上传三个附件（拖入即可）：
  - `release/JiweiToolbox-1.0.0-portable.exe`
  - `release/JiweiToolbox-1.0.0-setup.exe`
  - `release/SHA256_hash.txt`

发布后从页面实际下载一次，核对 SHA256 与 `SHA256_hash.txt` 一致。

## 5. 仓库设置建议

| 项目 | 建议值 |
|---|---|
| 可见性 | Public |
| Issues | 开启 |
| Discussions | 开启 |
| Wiki | 开启（结构见 `docs/WIKI.md`） |
| Topics | `windows` `tauri` `vue3` `toolbox` `system-utility` `uninstaller` `hardware-info` |
| Description | Windows 一站式系统运维工具箱：批量装机、深度卸载、硬件检测 |

## 6. 当前附件校验值

```
JiweiToolbox-1.0.0-portable.exe
  21740544 字节
  09ADA62DBFF7F10C6227E9EFF0E61686B5CE942602FC14846F204C12341673D8

JiweiToolbox-1.0.0-setup.exe
  8317344 字节
  01EC4A66D8EC0C9F01E37A3121681BF7B5F32A2DC8594CB41313702BC5FA29DC
```

注意：`release/*.exe` 已被 `.gitignore` 排除，附件只上传到 Release，不进仓库。
