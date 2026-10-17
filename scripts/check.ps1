# 极维工具箱一键检查：Rust 测试 + 前端类型检查 + 全量构建
$ErrorActionPreference = "Stop"
$env:PATH = "C:\Users\Administrator\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin;$env:PATH"
$env:HTTP_PROXY = "http://127.0.0.1:7890"; $env:HTTPS_PROXY = "http://127.0.0.1:7890"
Set-Location $PSScriptRoot\..

Write-Host "== cargo test ==" -ForegroundColor Cyan
Push-Location src-tauri; cargo test; Pop-Location
if ($LASTEXITCODE -ne 0) { Write-Host "cargo test 失败" -ForegroundColor Red; exit 1 }

Write-Host "== vue-tsc + vite build ==" -ForegroundColor Cyan
& "C:\Users\Administrator\.workbuddy\binaries\node\versions\22.22.2-6\npm.cmd" run build
if ($LASTEXITCODE -ne 0) { Write-Host "前端构建失败" -ForegroundColor Red; exit 1 }

Write-Host "== tauri build ==" -ForegroundColor Cyan
& "C:\Users\Administrator\.workbuddy\binaries\node\versions\22.22.2-6\npm.cmd" run tauri:build
if ($LASTEXITCODE -ne 0) { Write-Host "tauri build 失败" -ForegroundColor Red; exit 1 }

Write-Host "全部检查通过" -ForegroundColor Green
