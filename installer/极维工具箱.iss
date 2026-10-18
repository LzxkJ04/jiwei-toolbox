; ============================================================================
; 极维工具箱 安装脚本 (Inno Setup 7.1.0)
; 架构：Inno 负责安装逻辑（拷文件/注册表/快捷方式/卸载/静默），
;       WebView2 内嵌 HTML 向导负责全部视觉与交互，
;       JS 按钮经 32 位桥 DLL wv2bridge.dll 回调 Inno Pascal。
; 编译：C:\tools\inno\ISCC.exe "installer\极维工具箱.iss"
; 注意：本文件必须保存为 UTF-8 带 BOM。
; TODO: 以下文件当前为占位空文件（并行开发，编译可通过但运行需真实产物）：
;   ..\src-tauri\target\release\jiwei-toolbox.exe
;   res\bridge\wv2bridge.dll
;   res\ui.html
;   res\readme-license.txt / readme-config.txt / readme-backup.txt / readme-cache.txt
;   ..\src-tauri\icons\icon.ico（当前为最小合法 ICO，需替换为正式图标）
; ============================================================================

#define MyAppName "极维工具箱"
#define MyAppVersion "1.0.0"
#define MyAppExe "jiwei-toolbox.exe"

[Setup]
AppId={{8E7B6C2A-4D3F-4A1E-9C5B-2F6D8A0E1B7C}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppName}
; 硬编码 64 位 Program Files：32 位安装器的 {autopf}/{pf} 会展开为
; "C:\Program Files (x86)"，与主程序约定 C:\Program Files\极维工具箱 不符
DefaultDirName=C:\Program Files\{#MyAppName}
DefaultGroupName={#MyAppName}
PrivilegesRequired=admin
SetupIconFile=..\src-tauri\icons\icon.ico
Compression=lzma2
SolidCompression=yes
OutputDir=..\dist
OutputBaseFilename=jiwei-setup
; 安装包自身的版本信息，与 MyAppVersion 保持同步（属性页可见）
VersionInfoVersion={#MyAppVersion}
VersionInfoProductName={#MyAppName}
VersionInfoDescription={#MyAppName} 安装程序
VersionInfoCompany={#MyAppName}
VersionInfoCopyright=Copyright (C) 2026 {#MyAppName}
WizardStyle=modern
DisableProgramGroupPage=yes
; License 页仅在 WebView2 初始化失败的原生回退流程中显示
LicenseFile=res\readme-license.txt
UninstallDisplayName={#MyAppName}
UninstallDisplayIcon={app}\{#MyAppExe}
; Dirs 注册表五值、桌面/开始菜单快捷方式等按规格有意使用 HKCU（当前登录用户）
UsedUserAreasWarning=no

[Languages]
Name: "chs"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Files]
; 主程序
Source: "..\src-tauri\target\release\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion
; WebView2 桥 DLL 与 UI 页面（dontcopy，InitializeWizard 提取到 {tmp}）
Source: "res\bridge\wv2bridge.dll"; DestDir: "{tmp}"; Flags: dontcopy
Source: "res\ui.html"; DestDir: "{tmp}"; Flags: dontcopy
; README 说明文档（dontcopy，ssPostInstall 复制到用户派生目录）
Source: "res\readme-config.txt"; DestDir: "{tmp}"; Flags: dontcopy
Source: "res\readme-backup.txt"; DestDir: "{tmp}"; Flags: dontcopy
Source: "res\readme-cache.txt"; DestDir: "{tmp}"; Flags: dontcopy

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

[Code]
const
  BM_CLICK = 245;
  BridgeDll = '{tmp}\wv2bridge.dll';
  RegRoot = HKCU;
  RegDirsKey = 'Software\极维工具箱\Dirs';
  RegInstallerKey = 'Software\极维工具箱\Installer';

var
  bSilent: Boolean;          // 静默安装
  bWv2Failed: Boolean;       // WebView2 初始化失败 → 回退原生向导
  bWv2Started: Boolean;      // 已尝试初始化 WebView2（决定 DeinitializeSetup 是否 Destroy）
  bInstallStarted: Boolean;  // JS 已触发安装（防重复）
  bDesktop: Boolean;         // 桌面快捷方式开关
  bAdmin: Boolean;           // 管理员兼容层开关
  bKeepData: Boolean;        // 卸载时是否保留用户目录
  gProgramDir, gConfigDir, gBackupDir, gCacheDir, gLogDir: String;

// ---------------------------------------------------------------------------
// 桥 DLL 导入（delayload：首次调用时才加载，静默模式不触碰）
// ---------------------------------------------------------------------------
procedure WV2_Resize(w, h: Integer);
  external 'WV2_Resize@{tmp}\wv2bridge.dll stdcall delayload';
procedure WV2_Post(json: WideString);
  external 'WV2_Post@{tmp}\wv2bridge.dll stdcall delayload';
procedure WV2_Destroy();
  external 'WV2_Destroy@{tmp}\wv2bridge.dll stdcall delayload';
function WV2_Ready(): Integer;
  external 'WV2_Ready@{tmp}\wv2bridge.dll stdcall delayload';
function WV2_Browse(parent: HWND; title: WideString): WideString;
  external 'WV2_Browse@{tmp}\wv2bridge.dll stdcall delayload';
function WV2_PendingAction(): Integer;
  external 'WV2_PendingAction@{tmp}\wv2bridge.dll stdcall delayload';
function WV2_Field(key: WideString): WideString;
  external 'WV2_Field@{tmp}\wv2bridge.dll stdcall delayload';
procedure WV2_RegisterHandler(thunk: NativeInt);
  external 'WV2_RegisterHandler@{tmp}\wv2bridge.dll stdcall delayload';
// 兼容保留：旧方案用 BRIDGE_MakeThunk 适配调用约定；Inno 7 直接用
// CreateCallback(@Proc) 生成 stdcall 无参回调，桥侧按 stdcall(0 参数) 调用即可。
function BRIDGE_MakeThunk(proc: Integer; nargs: Integer): Integer;
  external 'BRIDGE_MakeThunk@{tmp}\wv2bridge.dll stdcall delayload';
function WV2_Init(parent: HWND; htmlPath: WideString): Integer;
  external 'WV2_Init@{tmp}\wv2bridge.dll stdcall delayload';
// 协议中 derive 动作所需（桥内计算派生 JSON，纯 ASCII \u 转义）
function WV2_Derive(dir: WideString): WideString;
  external 'WV2_Derive@{tmp}\wv2bridge.dll stdcall delayload';

// ---------------------------------------------------------------------------
// JSON 辅助
// ---------------------------------------------------------------------------
// 注意：CHR_NL/CHR_CR/CHR_TAB 用常量代替字面 #10/#13/#9，
// 避免行首的 # 被 ISPP 误识别为预处理指令。
const
  CHR_NL = #10;
  CHR_CR = #13;
  CHR_TAB = #9;
  INVALID_CHARS = '*?"<>|';
  HEXD = '0123456789ABCDEF';

function JsonEsc(const S: String): String;
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
      Result := Result + C;
    end;
  end;
end;

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

// 解码 JSON 字符串值中的转义（\\ \" \/ \n \r \t \uXXXX）
function DecodeJsonString(const S: String): String;
var
  I: Integer;
begin
  Result := '';
  I := 1;
  while I <= Length(S) do
  begin
    if S[I] = '\' then
    begin
      if I < Length(S) then
      begin
        case S[I + 1] of
          '"': begin Result := Result + '"'; I := I + 2; end;
          '\': begin Result := Result + '\'; I := I + 2; end;
          '/': begin Result := Result + '/'; I := I + 2; end;
          'n': begin Result := Result + CHR_NL; I := I + 2; end;
          'r': begin Result := Result + CHR_CR; I := I + 2; end;
          't': begin Result := Result + CHR_TAB; I := I + 2; end;
          'u':
            begin
              if I + 5 <= Length(S) then
              begin
                Result := Result + Chr(StrToIntDef('$' + Copy(S, I + 2, 4), 0));
                I := I + 6;
              end
              else
                I := I + 2;
            end;
        else
          begin Result := Result + S[I]; I := I + 1; end;
        end;
      end
      else
        I := I + 1;
    end
    else
    begin
      Result := Result + S[I];
      I := I + 1;
    end;
  end;
end;

// 从扁平 JSON 对象中提取字符串字段值（不依赖键顺序；值须为带引号字符串）
function ExtractJsonValue(const Json, Key: String): String;
var
  I, N: Integer;
  Pat: String;
begin
  Result := '';
  Pat := '"' + Key + '"';
  I := Pos(Pat, Json);
  if I = 0 then Exit;
  I := I + Length(Pat);
  N := Length(Json);
  while (I <= N) and (Json[I] <= ' ') do I := I + 1;
  if (I > N) or (Json[I] <> ':') then Exit;
  I := I + 1;
  while (I <= N) and (Json[I] <= ' ') do I := I + 1;
  if (I > N) or (Json[I] <> '"') then Exit;
  I := I + 1;
  while I <= N do
  begin
    if Json[I] = '\' then
    begin
      // 保留转义序列，最后统一解码
      Result := Result + Json[I];
      if I + 1 <= N then
      begin
        Result := Result + Json[I + 1];
        I := I + 2;
      end
      else
        I := I + 1;
    end
    else if Json[I] = '"' then
      Break
    else
    begin
      Result := Result + Json[I];
      I := I + 1;
    end;
  end;
  Result := DecodeJsonString(Result);
end;

// ---------------------------------------------------------------------------
// 目录校验与派生
// ---------------------------------------------------------------------------
function IsValidInstallDir(const Dir: String): Boolean;
var
  I: Integer;
  U: String;
begin
  Result := False;
  if Length(Dir) < 4 then Exit;                       // 长度 >= 4
  for I := 1 to Length(Dir) do                        // 非法字符
    if Pos(Dir[I], INVALID_CHARS) > 0 then Exit;
  if Pos(':\', Uppercase(Dir)) < 2 then Exit;         // 须为形如 X:\... 的本地路径
  U := Uppercase(AddBackslash(Dir));
  // 系统保护目录：Windows 目录与系统目录子树、盘符根目录
  if Pos(Uppercase(AddBackslash(ExpandConstant('{win}'))), U) = 1 then Exit;
  if Pos(Uppercase(AddBackslash(ExpandConstant('{sys}'))), U) = 1 then Exit;
  if U = Uppercase(AddBackslash(ExpandConstant('{sd}'))) then Exit;
  Result := True;
end;

// 用桥的派生结果（含 config/backup/cache）填充全局目录变量，失败/缺省时回退默认值
procedure ApplyDirs(const InstallDir: String);
var
  DeriveJson: String;
begin
  gProgramDir := InstallDir;
  DeriveJson := '';
  if not bSilent and not bWv2Failed then
  begin
    try
      DeriveJson := WV2_Derive(InstallDir);
    except
      DeriveJson := '';
    end;
  end;
  gConfigDir := ExtractJsonValue(DeriveJson, 'config');
  gBackupDir := ExtractJsonValue(DeriveJson, 'backup');
  gCacheDir  := ExtractJsonValue(DeriveJson, 'cache');
  gLogDir    := ExtractJsonValue(DeriveJson, 'log');
  if gLogDir = '' then gLogDir := ExtractJsonValue(DeriveJson, 'logs');
  if gConfigDir = '' then gConfigDir := ExpandConstant('{userdocs}\极维工具箱\配置');
  if gBackupDir = '' then gBackupDir := ExpandConstant('{userdocs}\极维工具箱\备份导出');
  if gCacheDir  = '' then gCacheDir  := ExpandConstant('{localappdata}\极维工具箱\缓存');
  if gLogDir    = '' then gLogDir    := ExpandConstant('{localappdata}\极维工具箱\缓存\logs');
  // 写注册表 Dirs 五值
  RegWriteStringValue(RegRoot, RegDirsKey, 'Program', gProgramDir);
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
// WebView2 JS → Pascal 动作处理（无参，经桥 thunk 回调）
// ---------------------------------------------------------------------------
procedure OnJsAction();
var
  Action: Integer;
  Dir, P: String;
  ec: Integer;
begin
  Action := WV2_PendingAction();
  case Action of
    1: // derive：桥已算好派生 JSON，转发给 JS
      begin
        Dir := WV2_Field('dir');
        WV2_Post(WV2_Derive(Dir));
      end;
    2: // install：校验目录 → 保存 Dirs → 模拟点击 Next 进入 wpInstalling
      begin
        if bInstallStarted then Exit;
        Dir := WV2_Field('dir');
        if not IsValidInstallDir(Dir) then
          WV2_Post('{"type":"error","msg":"安装目录不合法：长度至少 4 个字符，不能包含 *?\"<>| ，也不能位于 Windows 系统目录内。"}')
        else
        begin
          bInstallStarted := True;
          bDesktop := (WV2_Field('desktop') = 'true');
          bAdmin   := (WV2_Field('admin') = 'true');
          WizardForm.DirEdit.Text := Dir;   // 使 {app} 指向用户选择的目录
          ApplyDirs(Dir);
          // 延迟点击，让本桥回调先返回
          PostMessage(WizardForm.NextButton.Handle, BM_CLICK, 0, 0);
        end;
      end;
    3: // chooseDir：弹出系统目录选择对话框
      begin
        P := WV2_Browse(WizardForm.Handle, '选择安装目录');
        if P <> '' then
          WV2_Post('{"type":"dir","dir":"' + JsonEsc(P) + '"}');
      end;
    4: // finish
      begin
        if WV2_Field('run') = 'true' then
          Exec(ExpandConstant('{app}\jiwei-toolbox.exe'), '', ExpandConstant('{app}'), SW_SHOW, ewNoWait, ec);
        if WV2_Field('openCfg') = 'true' then
          Exec('explorer.exe', gConfigDir, '', SW_SHOW, ewNoWait, ec);
      end;
    5: // confirmExit
      begin
        if MsgBox('确定要退出安装向导吗？', mbConfirmation, MB_YESNO) = IDYES then
          WizardForm.Close;
      end;
  end;
end;

// ---------------------------------------------------------------------------
// 安装流程
// ---------------------------------------------------------------------------
function InitializeSetup(): Boolean;
begin
  bSilent := WizardSilent();
  bWv2Failed := False;
  bWv2Started := False;
  bInstallStarted := False;
  bDesktop := True;   // 静默默认：创建桌面快捷方式
  bAdmin := False;
  bKeepData := True;
  // 卸载参数 /KEEPDATA=0 → 安装时写 KeepData=0
  if Pos('/KEEPDATA=0', Uppercase(GetCmdTail())) > 0 then
    bKeepData := False;
  // 默认派生目录（静默模式或桥派生缺失时的回退值；与主程序约定一致，中文目录名）
  // program 硬编码 64 位 Program Files：32 位安装器的 {autopf}/{pf} 展开为 x86 路径
  // 静默模式下若有 /DIR= 参数，ssPostInstall 阶段会用 WizardDirValue() 覆盖（此时
  // WizardForm 已创建；InitializeSetup 阶段调用会报 WizardForm 未创建的内部错误）
  gProgramDir := 'C:\Program Files\极维工具箱';
  gConfigDir := ExpandConstant('{userdocs}\极维工具箱\配置');
  gBackupDir := ExpandConstant('{userdocs}\极维工具箱\备份导出');
  gCacheDir := ExpandConstant('{localappdata}\极维工具箱\缓存');
  gLogDir := ExpandConstant('{localappdata}\极维工具箱\缓存\logs');
  Result := True;
end;

procedure InitializeWizard();
var
  I, rc: Integer;
begin
  WizardForm.BorderStyle := bsDialog;
  WizardForm.BorderIcons := [biSystemMenu];
  WizardForm.Position := poScreenCenter;
  WizardForm.ClientWidth := 780;
  WizardForm.ClientHeight := 540;

  if bSilent then Exit; // 静默：不提取、不初始化 WebView2

  // WebView2 内嵌 UI（失败则回退 Inno 原生页面）
  try
    ExtractTemporaryFile('wv2bridge.dll');
    ExtractTemporaryFile('ui.html');
    rc := WV2_Init(WizardForm.Handle, ExpandConstant('{tmp}\ui.html'));
    bWv2Started := True;
    if rc <> 0 then
      bWv2Failed := True
    else
    begin
      WV2_Resize(780, 540); // 桥在 put_Bounds 后自监听父窗口 WM_SIZE
      for I := 0 to 100 do
      begin
        Sleep(50);
        if WV2_Ready() = 1 then Break;
      end;
      if WV2_Ready() = 1 then
      begin
        WV2_RegisterHandler(CreateCallback(@OnJsAction));
        WV2_Post('{"type":"hello","defaultDir":"' +
          JsonEsc('C:\Program Files\极维工具箱') + '"}');
      end
      else
        bWv2Failed := True; // 5s 未就绪，回退原生
    end;
  except
    bWv2Failed := True;   // 桥 DLL 加载/初始化异常，回退原生
  end;
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  // WebView2 覆盖向导：跳过全部交互页，打开后直接进入 wpInstalling；
  // 回退（bWv2Failed）时保留 Inno 原生 Welcome/License/Dir/Ready 页面
  if bWv2Failed then
    Result := False
  else
    Result := (PageID = wpWelcome) or (PageID = wpLicense) or
              (PageID = wpSelectDir) or (PageID = wpSelectProgramGroup) or
              (PageID = wpReady) or (PageID = wpPreparing);
end;

procedure CurProgressChanged(Progress: Integer);
begin
  if not bSilent and not bWv2Failed and bWv2Started then
    WV2_Post(Format('{"type":"progress","pct":%d,"status":"正在复制文件…"}', [Progress]));
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
        if not bSilent and not bWv2Failed and bWv2Started then
          WV2_Post('{"type":"progress","pct":0,"status":"正在准备安装…"}');
      end;
    ssPostInstall:
      begin
        // 静默模式：WizardDirValue 返回 /DIR= 指定目录；未指定时即硬编码默认值
        if bSilent then
          gProgramDir := RemoveBackslashUnlessRoot(WizardDirValue());
        // dirs.json（无 BOM 的 UTF-8，供主程序读取各派生目录）
        // 方案：先用 JsonEscUnicode 转成纯 ASCII JSON，再用 SaveStringToFile
        // 原样写字节流——纯 ASCII 即合法 UTF-8 且天然无 BOM
        //（SaveStringsToUTF8File 会写 BOM，导致部分 JSON 解析器失败，故弃用）
        S := '{"program":"' + JsonEscUnicode(gProgramDir) + '"' +
             ',"config":"' + JsonEscUnicode(gConfigDir) + '"' +
             ',"backup":"' + JsonEscUnicode(gBackupDir) + '"' +
             ',"cache":"' + JsonEscUnicode(gCacheDir) + '"' +
             ',"log":"' + JsonEscUnicode(gLogDir) + '"}';
        SaveStringToFile(ExpandConstant('{app}\dirs.json'), S, False);

        // README 静态文档复制到派生目录
        try
          ForceDirectories(gConfigDir);
          ForceDirectories(gBackupDir);
          ForceDirectories(gCacheDir);
          ExtractTemporaryFile('readme-config.txt');
          ExtractTemporaryFile('readme-backup.txt');
          ExtractTemporaryFile('readme-cache.txt');
          CopyFile(ExpandConstant('{tmp}\readme-config.txt'), AddBackslash(gConfigDir) + 'README.txt', False);
          CopyFile(ExpandConstant('{tmp}\readme-backup.txt'), AddBackslash(gBackupDir) + 'README.txt', False);
          CopyFile(ExpandConstant('{tmp}\readme-cache.txt'), AddBackslash(gCacheDir) + 'README.txt', False);
        except
          // 复制失败不阻断安装
        end;

        // 快捷方式：开始菜单 + 桌面（开关）
        try
          ForceDirectories(ExpandConstant('{userprograms}\极维工具箱'));
          CreateShellLink(ExpandConstant('{userprograms}\极维工具箱\极维工具箱.lnk'),
            '极维工具箱', ExpandConstant('{app}\jiwei-toolbox.exe'), '',
            ExpandConstant('{app}'), '', 0, SW_SHOWNORMAL);
          if bDesktop then
            CreateShellLink(ExpandConstant('{userdesktop}\极维工具箱.lnk'),
              '极维工具箱', ExpandConstant('{app}\jiwei-toolbox.exe'), '',
              ExpandConstant('{app}'), '', 0, SW_SHOWNORMAL);
        except
        end;

        // 管理员兼容层（开关）
        if bAdmin then
          RegWriteStringValue(HKCU,
            'SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers',
            ExpandConstant('{app}\jiwei-toolbox.exe'), '~ RUNASADMIN');

        if not bSilent and not bWv2Failed and bWv2Started then
          WV2_Post('{"type":"progress","pct":100,"status":"安装完成"}');
      end;
  end;
end;

procedure DeinitializeSetup();
begin
  if bWv2Started then
  begin
    try
      WV2_Destroy();
    except
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
  // 询问用户最终选择
  if MsgBox('是否保留用户配置与备份目录？' #13#10 +
            '「是」= 保留配置、备份与缓存目录；「否」= 卸载时一并删除。',
            mbConfirmation, MB_YESNO) = IDNO then
    bKeepData := False;
  // 清理管理员兼容层
  RegDeleteValue(HKCU,
    'SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers',
    ExpandConstant('{app}\jiwei-toolbox.exe'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if not bKeepData then
    begin
      DelTree(GetDirsValue('Config'), True, True, True);
      DelTree(GetDirsValue('Backup'), True, True, True);
      DelTree(GetDirsValue('Cache'), True, True, True);
      DelTree(GetDirsValue('Log'), True, True, True);
    end;
  end;
end;
