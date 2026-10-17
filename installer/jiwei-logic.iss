; ============================================================================
; 极维工具箱 纯静默逻辑包 (Inno Setup 7.1.0)
; 架构：Tauri 单 exe 三模式（主程序 / --installer 安装向导 / --uninstaller 卸载器）。
;       Tauri 壳负责全部 UI（协议/目录选择/进度/完成），并自行拷贝主程序 payload
;       （壳复制自身 exe 到目标目录）。本包只负责静默执行安装逻辑：
;       注册表、卸载器生成、快捷方式、dirs.json、关联。
; 调用：jiwei-logic.exe /VERYSILENT /DIR=<目标> [/DATA=<用户数据目录>] /DESKTOP=1 /STARTMENU=1 /ADMIN=1 [/KEEPDATA=0]
; 编译：C:\tools\inno\ISCC.exe "installer\jiwei-logic.iss"
; 注意：本文件必须保存为 UTF-8 带 BOM。
; ============================================================================

#define MyAppName "极维工具箱"
#define MyAppVersion "0.1.0"
#define MyAppExe "jiwei-toolbox.exe"

[Setup]
AppId={{8E7B6C2A-4D3F-4A1E-9C5B-2F6D8A0E1B7C}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppName}
; 硬编码 64 位 Program Files：32 位安装器的 {autopf}/{pf} 会展开为
; "C:\Program Files (x86)"，与主程序约定 C:\Program Files\极维工具箱 不符
;（静默安装时壳以 /DIR= 覆盖，Inno 原生支持）
DefaultDirName=C:\Program Files\{#MyAppName}
DefaultGroupName={#MyAppName}
PrivilegesRequired=admin
SetupIconFile=..\src-tauri\icons\icon.ico
Compression=lzma2
SolidCompression=yes
OutputDir=..\dist-logic
OutputBaseFilename=jiwei-logic
WizardStyle=modern
DisableProgramGroupPage=yes
; 纯静默逻辑包：任何情况下不得出现 UI
DisableWelcomePage=yes
DisableStartupPrompt=yes
DisableDirPage=yes
DisableReadyPage=yes
DisableFinishedPage=yes
UninstallDisplayName={#MyAppName}
UninstallDisplayIcon={app}\{#MyAppExe}
; Dirs 注册表五值、桌面/开始菜单快捷方式等按规格有意使用 HKCU（当前登录用户）
UsedUserAreasWarning=no

[Languages]
Name: "chs"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Registry]
; ---- 文件关联 .jwbak（HKLM64 避免 32 位安装器的 Wow6432Node 重定向）----
Root: HKLM64; Subkey: "Software\Classes\.jwbak"; ValueType: string; ValueName: ""; ValueData: "JiweiToolbox.Backup"; Flags: uninsdeletekey
Root: HKLM64; Subkey: "Software\Classes\JiweiToolbox.Backup"; ValueType: string; ValueName: ""; ValueData: "极维工具箱备份文件"; Flags: uninsdeletekey
Root: HKLM64; Subkey: "Software\Classes\JiweiToolbox.Backup\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#MyAppExe},0"; Flags: uninsdeletekey
Root: HKLM64; Subkey: "Software\Classes\JiweiToolbox.Backup\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExe}"" ""%1"""; Flags: uninsdeletekey
; ---- 右键菜单：扫描软件残留 ----
Root: HKLM64; Subkey: "Software\Classes\*\shell\JiweiScan"; ValueType: string; ValueName: ""; ValueData: "用极维工具箱扫描软件残留"; Flags: uninsdeletekey
Root: HKLM64; Subkey: "Software\Classes\*\shell\JiweiScan\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExe}"" --scan ""%1"""; Flags: uninsdeletekey
; ---- 卸载保留数据标记（默认 1=保留，安装时 /KEEPDATA=0 可改写；InitializeUninstall 读取）----
Root: HKCU; Subkey: "Software\极维工具箱\Installer"; ValueType: dword; ValueName: "KeepData"; ValueData: "$1"; Flags: uninsdeletekey

; 主程序 payload jiwei-toolbox.exe 由 Tauri 壳自行拷贝到 {app}，不在本包清单内；
; Inno 只卸载它装过的文件，故在 [UninstallDelete] 补删，避免卸载残留。
[UninstallDelete]
Type: files; Name: "{app}\{#MyAppExe}"
Type: files; Name: "{app}\dirs.json"

[Code]
const
  RegRoot = HKCU;
  RegDirsKey = 'Software\极维工具箱\Dirs';
  RegInstallerKey = 'Software\极维工具箱\Installer';

var
  bSilent: Boolean;          // 静默安装
  bDesktop: Boolean;         // 桌面快捷方式开关（/DESKTOP=1，默认 1）
  bStartMenu: Boolean;       // 开始菜单快捷方式开关（/STARTMENU=1，默认 1）
  bAdmin: Boolean;           // 管理员兼容层开关（/ADMIN=1，默认 0）
  bKeepData: Boolean;        // 卸载时是否保留用户目录
  gDataParam: String;        // /DATA= 用户数据目录（空 = 走默认派生）
  gProgramDir, gDataDir, gConfigDir, gBackupDir, gCacheDir, gLogDir: String;

// ---------------------------------------------------------------------------
// JSON 辅助
// ---------------------------------------------------------------------------
// 注意：CHR_NL/CHR_CR/CHR_TAB 用常量代替字面 #10/#13/#9，
// 避免行首的 # 被 ISPP 误识别为预处理指令。
const
  CHR_NL = #10;
  CHR_CR = #13;
  CHR_TAB = #9;
  HEXD = '0123456789ABCDEF';

// 把 0..65535 转为 4 位大写十六进制（Inno PS 无 IntToHex）
function Hex4(N: Integer): String;
begin
  Result := HEXD[(N shr 12) and 15 + 1] + HEXD[(N shr 8) and 15 + 1] +
            HEXD[(N shr 4) and 15 + 1] + HEXD[N and 15 + 1];
end;

// 把内容转为纯 ASCII JSON 字符串：控制字符/引号按 JSON 规则转义，
// 非 ASCII 字符统一转 \uXXXX（UTF-16 代理对逐单元转义同样合法）。
// 用于 SaveStringToFile 写无 BOM 的 UTF-8 兼容文件（纯 ASCII 字节即 UTF-8）。
function JsonEscUnicode(const S: String): String;
var
  I: Integer;
  C: Char;
begin
  Result := '';
  for I := 1 to Length(S) do
  begin
    C := S[I];
    case C of
      '\': Result := Result + '\\';
      '"': Result := Result + '\"';
      CHR_NL: Result := Result + '\n';
      CHR_CR: Result := Result + '\r';
      CHR_TAB: Result := Result + '\t';
    else
      if Ord(C) < 128 then
        Result := Result + C
      else
        Result := Result + '\u' + Hex4(Ord(C));
    end;
  end;
end;

// ---------------------------------------------------------------------------
// 目录派生（纯静默：仅默认值，与主程序约定一致，中文目录名）
// ---------------------------------------------------------------------------
procedure ApplyDirs(const InstallDir: String);
begin
  gProgramDir := InstallDir;
  // 双路径模型：/DATA= 非空 → config/backup/cache/log 全部挂到该数据目录下；
  // 为空 → 保持旧默认派生（Documents + %LOCALAPPDATA%）
  gDataParam := RemoveBackslashUnlessRoot(gDataParam);
  if gDataParam <> '' then
  begin
    gDataDir   := gDataParam;
    gConfigDir := gDataParam + '\配置';
    gBackupDir := gDataParam + '\备份导出';
    gCacheDir  := gDataParam + '\缓存';
    gLogDir    := gDataParam + '\缓存\logs';
  end
  else
  begin
    gDataDir   := ExpandConstant('{localappdata}\极维工具箱');
    gConfigDir := ExpandConstant('{userdocs}\极维工具箱\配置');
    gBackupDir := ExpandConstant('{userdocs}\极维工具箱\备份导出');
    gCacheDir  := ExpandConstant('{localappdata}\极维工具箱\缓存');
    gLogDir    := ExpandConstant('{localappdata}\极维工具箱\缓存\logs');
  end;
  // 写注册表 Dirs 六值
  RegWriteStringValue(RegRoot, RegDirsKey, 'Program', gProgramDir);
  RegWriteStringValue(RegRoot, RegDirsKey, 'Data',    gDataDir);
  RegWriteStringValue(RegRoot, RegDirsKey, 'Config',  gConfigDir);
  RegWriteStringValue(RegRoot, RegDirsKey, 'Backup',  gBackupDir);
  RegWriteStringValue(RegRoot, RegDirsKey, 'Cache',   gCacheDir);
  RegWriteStringValue(RegRoot, RegDirsKey, 'Log',     gLogDir);
end;

function GetDirsValue(const Name: String): String;
begin
  if not RegQueryStringValue(RegRoot, RegDirsKey, Name, Result) then
    Result := '';
end;

// ---------------------------------------------------------------------------
// 安装流程
// ---------------------------------------------------------------------------
function InitializeSetup(): Boolean;
begin
  bSilent := WizardSilent();
  bKeepData := True;
  // 卸载参数 /KEEPDATA=0 → 安装时写 KeepData=0
  if Pos('/KEEPDATA=0', Uppercase(GetCmdTail())) > 0 then
    bKeepData := False;
  // 快捷方式/兼容层开关（壳以命令行参数传入；{param:...} 静默原生支持）
  bDesktop := ExpandConstant('{param:DESKTOP|1}') <> '0';
  bStartMenu := ExpandConstant('{param:STARTMENU|1}') <> '0';
  bAdmin := ExpandConstant('{param:ADMIN|0}') = '1';
  // 用户数据目录（/DATA=，空字符串缺省 = 默认派生）
  gDataParam := ExpandConstant('{param:DATA|}');
  Result := True;
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  // 纯静默逻辑包：跳过全部交互页（保留 wpInstalling 使安装照常进行）
  Result := PageID <> wpInstalling;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  S: String;
begin
  case CurStep of
    ssInstall:
      begin
        // KeepData 标记（默认 1，/KEEPDATA=0 时为 0；卸载时读取）
        RegWriteDWordValue(RegRoot, RegInstallerKey, 'KeepData', Ord(bKeepData));
      end;
    ssPostInstall:
      begin
        // WizardDirValue 返回 /DIR= 指定目录；未指定时即硬编码默认值
        gProgramDir := RemoveBackslashUnlessRoot(WizardDirValue());
        ApplyDirs(gProgramDir);

        // dirs.json（无 BOM 的 UTF-8，供主程序读取各派生目录）
        // 方案：先用 JsonEscUnicode 转成纯 ASCII JSON，再用 SaveStringToFile
        // 原样写字节流——纯 ASCII 即合法 UTF-8 且天然无 BOM
        //（SaveStringsToUTF8File 会写 BOM，导致部分 JSON 解析器失败，故弃用）
        S := '{"program":"' + JsonEscUnicode(gProgramDir) + '"' +
             ',"data":"' + JsonEscUnicode(gDataDir) + '"' +
             ',"config":"' + JsonEscUnicode(gConfigDir) + '"' +
             ',"backup":"' + JsonEscUnicode(gBackupDir) + '"' +
             ',"cache":"' + JsonEscUnicode(gCacheDir) + '"' +
             ',"log":"' + JsonEscUnicode(gLogDir) + '"}';
        SaveStringToFile(ExpandConstant('{app}\dirs.json'), S, False);

        // 快捷方式：开始菜单（开关）+ 桌面（开关）
        try
          if bStartMenu then
          begin
            ForceDirectories(ExpandConstant('{userprograms}\极维工具箱'));
            CreateShellLink(ExpandConstant('{userprograms}\极维工具箱\极维工具箱.lnk'),
              '极维工具箱', ExpandConstant('{app}\{#MyAppExe}'), '',
              ExpandConstant('{app}'), '', 0, SW_SHOWNORMAL);
          end;
          if bDesktop then
            CreateShellLink(ExpandConstant('{userdesktop}\极维工具箱.lnk'),
              '极维工具箱', ExpandConstant('{app}\{#MyAppExe}'), '',
              ExpandConstant('{app}'), '', 0, SW_SHOWNORMAL);
        except
        end;

        // 管理员兼容层（开关）
        if bAdmin then
          RegWriteStringValue(HKCU,
            'SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers',
            ExpandConstant('{app}\{#MyAppExe}'), '~ RUNASADMIN');
      end;
  end;
end;

// ---------------------------------------------------------------------------
// 卸载
// ---------------------------------------------------------------------------
function InitializeUninstall(): Boolean;
var
  N: Cardinal;
begin
  Result := True;
  // 读取安装时写入的 KeepData（缺省视为保留）
  bKeepData := True;
  if RegQueryDWordValue(HKCU, RegInstallerKey, 'KeepData', N) then
    bKeepData := (N <> 0);
  // 静默卸载时 MsgBox 会阻塞等待输入，仅在交互模式询问；
  // 静默时按注册表 KeepData 值走。
  // 注意：Inno 7.1.0 禁止在卸载阶段调用 WizardSilent()（内部错误致命中止），
  // 故解析命令行判断静默（/VERYSILENT 不含子串 /SILENT，两者需分别检测）
  if (Pos('/VERYSILENT', Uppercase(GetCmdTail())) = 0) and
     (Pos('/SILENT', Uppercase(GetCmdTail())) = 0) then
    if MsgBox('是否保留用户配置与备份目录？' #13#10 +
              '「是」= 保留配置、备份与缓存目录；「否」= 卸载时一并删除。',
              mbConfirmation, MB_YESNO) = IDNO then
      bKeepData := False;
  // 清理管理员兼容层
  RegDeleteValue(HKCU,
    'SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers',
    ExpandConstant('{app}\{#MyAppExe}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if not bKeepData then
    begin
      // 自定义数据目录（/DATA=）非空时整个删除；再兜底删旧派生目录
      DelTree(GetDirsValue('Data'), True, True, True);
      DelTree(GetDirsValue('Config'), True, True, True);
      DelTree(GetDirsValue('Backup'), True, True, True);
      DelTree(GetDirsValue('Cache'), True, True, True);
      DelTree(GetDirsValue('Log'), True, True, True);
    end;
  end;
end;
