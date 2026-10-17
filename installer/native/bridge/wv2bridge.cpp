// wv2bridge.cpp — 32 位 WebView2 桥 DLL，供 Inno Setup (32 位) Pascal Script 调用。
// 单例桥：内部独立 STA 桥线程创建 WebView2，通过子类化 Inno 向导窗口的
// WM_TIMER 泵把 JS→宿主动作派发回 Inno 主线程。
// 本文件必须以 UTF-8 带 BOM 保存（MSVC 才能正确读取中文注释/字符串）。

#ifndef UNICODE
#define UNICODE
#endif
#ifndef _UNICODE
#define _UNICODE
#endif

#include <windows.h>
#include <objbase.h>
#include <shlobj.h>
#include <shobjidl.h>
#include <string>
#include <map>
#include <cstdio>
#include <cwchar>

#include <WebView2.h>

// ---------------------------------------------------------------------------
// 全局状态
// ---------------------------------------------------------------------------

static CRITICAL_SECTION g_cs;
static volatile LONG    g_csInited = 0;

static HANDLE g_thread    = nullptr;   // 桥线程句柄
static DWORD  g_threadId  = 0;
static HWND   g_parent    = nullptr;   // Inno 向导窗口
static std::wstring g_url;             // file:/// URL

static ICoreWebView2Environment* g_env        = nullptr;
static ICoreWebView2Controller*  g_controller = nullptr;
static ICoreWebView2*            g_webview    = nullptr;

static bool g_ctlReady = false;   // controller 创建完成
static bool g_navReady = false;   // 页面导航完成
static int  g_pending  = 0;       // 最近一条未消费的 JS→宿主动作（1..5）

static std::map<std::wstring, std::wstring> g_fields; // 最近消息字段表
static void*    g_thunk    = nullptr;  // Pascal 处理过程 thunk
static LONG_PTR g_origProc = 0;        // 子类化前的原窗口过程

static const UINT WM_APP_POST   = WM_APP + 1; // lParam=BSTR：PostWebMessageAsJson
static const UINT WM_APP_RESIZE = WM_APP + 2; // wParam=w, lParam=h
static const UINT TIMER_PUMP    = 3001;       // 主线程动作泵定时器

static const int ACTION_NONE        = 0;
static const int ACTION_DERIVE      = 1;
static const int ACTION_INSTALL     = 2;
static const int ACTION_CHOOSE_DIR  = 3;
static const int ACTION_FINISH      = 4;
static const int ACTION_CONFIRMEXIT = 5;

static void LockCS()  { EnterCriticalSection(&g_cs); }
static void UnlockCS(){ LeaveCriticalSection(&g_cs); }

static void EnsureCS()
{
    // WV2_Init/Destroy/Browse 均在 Inno 主线程调用，无并发初始化风险
    if (!g_csInited) {
        InitializeCriticalSection(&g_cs);
        InterlockedExchange(&g_csInited, 1);
    }
}

// ---------------------------------------------------------------------------
// JSON 简单解析（状态机扫描字符串字段，含 \" \\ \uXXXX 等转义还原）
// ---------------------------------------------------------------------------

// 从 s[i]（应指向 '"'）解析一个 JSON 字符串到 out，i 推进到字符串结束之后。
static bool ParseJsonString(const std::wstring& s, size_t& i, std::wstring& out)
{
    const size_t n = s.size();
    if (i >= n || s[i] != L'"') return false;
    ++i;
    out.clear();
    while (i < n) {
        wchar_t c = s[i];
        if (c == L'"') { ++i; return true; }
        if (c == L'\\') {
            ++i;
            if (i >= n) return false;
            wchar_t e = s[i];
            switch (e) {
                case L'"':  out += L'"';  break;
                case L'\\': out += L'\\'; break;
                case L'/':  out += L'/';  break;
                case L'b':  out += L'\b'; break;
                case L'f':  out += L'\f'; break;
                case L'n':  out += L'\n'; break;
                case L'r':  out += L'\r'; break;
                case L't':  out += L'\t'; break;
                case L'u': {
                    if (i + 4 >= n) return false;
                    unsigned v = 0;
                    for (int k = 1; k <= 4; ++k) {
                        wchar_t h = s[i + k];
                        v <<= 4;
                        if      (h >= L'0' && h <= L'9') v += (unsigned)(h - L'0');
                        else if (h >= L'a' && h <= L'f') v += (unsigned)(h - L'a' + 10);
                        else if (h >= L'A' && h <= L'F') v += (unsigned)(h - L'A' + 10);
                        else return false;
                    }
                    out += (wchar_t)v;
                    i += 4;
                    break;
                }
                default: out += e; break;
            }
            ++i;
        } else {
            out += c;
            ++i;
        }
    }
    return false;
}

// 在 json 文本中查找 "key" 对应的字符串值（简单扫描，够用即可）
static bool JsonGetString(const std::wstring& json, const wchar_t* key, std::wstring& out)
{
    const size_t n = json.size();
    size_t i = 0;
    while (i < n) {
        if (json[i] == L'"') {
            std::wstring k;
            if (!ParseJsonString(json, i, k)) return false;
            while (i < n && iswspace(json[i])) ++i;
            if (i < n && json[i] == L':') {
                ++i;
                while (i < n && iswspace(json[i])) ++i;
                if (i < n && json[i] == L'"') {
                    std::wstring v;
                    if (!ParseJsonString(json, i, v)) return false;
                    if (k == key) { out = v; return true; }
                } else {
                    // 非字符串标量值：跳过到 ',' 或 '}'
                    while (i < n && json[i] != L',' && json[i] != L'}') ++i;
                }
            }
            // 否则该字符串是数组元素或值，继续扫描
        } else {
            ++i;
        }
    }
    return false;
}

// ---------------------------------------------------------------------------
// JSON 输出转义（所有非 ASCII → \uXXXX，保证纯 ASCII；反斜杠 → \\）
// ---------------------------------------------------------------------------

static std::string EscJson(const std::wstring& v)
{
    std::string o;
    o.reserve(v.size() * 2 + 8);
    wchar_t buf[8];
    for (size_t idx = 0; idx < v.size(); ++idx) {
        wchar_t c = v[idx];
        if (c == L'\\')      o += "\\\\";
        else if (c == L'"')  o += "\\\"";
        else if (c >= 0x20 && c < 0x80) o += (char)c;
        else {
            swprintf(buf, 8, L"\\u%04x", (unsigned)(unsigned short)c);
            for (const wchar_t* q = buf; *q; ++q) o += (char)*q; // 输出恒为 ASCII
        }
    }
    return o;
}

static std::wstring AsciiToWide(const std::string& s)
{
    return std::wstring(s.begin(), s.end()); // 输入保证纯 ASCII
}

// ---------------------------------------------------------------------------
// 派生路径与磁盘剩余空间 → deriveJson
// ---------------------------------------------------------------------------

static std::wstring PathJoin(const std::wstring& a, const std::wstring& b)
{
    if (a.empty()) return b;
    if (b.empty()) return a;
    wchar_t la = a[a.size() - 1];
    if (b[0] == L'\\' || b[0] == L'/' || la == L'\\' || la == L'/') return a + b;
    return a + L"\\" + b;
}

// 返回纯 ASCII 的 JSON：
// {"config":"...","backup":"...","cache":"...","log":"...","disk":"目标磁盘剩余空间：123 GB"}
static std::string BuildDeriveJson(const std::wstring& dir)
{
    wchar_t buf[MAX_PATH];
    std::wstring sDocs, sLad;
    if (SUCCEEDED(SHGetFolderPathW(nullptr, CSIDL_PERSONAL, nullptr, SHGFP_TYPE_CURRENT, buf)))
        sDocs = buf;
    else
        sDocs = L"C:";
    if (SUCCEEDED(SHGetFolderPathW(nullptr, CSIDL_LOCAL_APPDATA, nullptr, SHGFP_TYPE_CURRENT, buf)))
        sLad = buf;
    else
        sLad = L"C:";

    std::wstring config = PathJoin(sDocs, L"极维工具箱\\配置");
    std::wstring backup = PathJoin(sDocs, L"极维工具箱\\备份导出");
    std::wstring cache  = PathJoin(sLad,  L"极维工具箱\\缓存");
    std::wstring log    = PathJoin(cache, L"logs");

    // 目标目录所在盘剩余空间（对调用方给定的目录查询其卷）
    ULARGE_INTEGER freeBytes;
    freeBytes.QuadPart = 0;
    std::wstring target = dir.empty() ? std::wstring(L"C:\\") : dir;
    GetDiskFreeSpaceExW(target.c_str(), &freeBytes, nullptr, nullptr);
    unsigned long long gb = freeBytes.QuadPart / (1024ULL * 1024ULL * 1024ULL);
    wchar_t diskStr[128];
    swprintf(diskStr, 128, L"目标磁盘剩余空间：%I64u GB", gb);

    std::string j = "{";
    j += "\"config\":\"" + EscJson(config) + "\",";
    j += "\"backup\":\"" + EscJson(backup) + "\",";
    j += "\"cache\":\""  + EscJson(cache)  + "\",";
    j += "\"log\":\""    + EscJson(log)    + "\",";
    j += "\"disk\":\""   + EscJson(diskStr)+ "\"";
    j += "}";
    return j;
}

// ---------------------------------------------------------------------------
// COM 事件 Handler（简单引用计数类）
// ---------------------------------------------------------------------------

class EnvCompletedHandler : public ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler {
    LONG m_ref;
public:
    EnvCompletedHandler() : m_ref(1) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        if (riid == IID_IUnknown ||
            riid == IID_ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler) {
            *ppv = static_cast<ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler*>(this);
            AddRef(); return S_OK;
        }
        *ppv = nullptr; return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override  { return InterlockedIncrement(&m_ref); }
    ULONG STDMETHODCALLTYPE Release() override {
        ULONG r = InterlockedDecrement(&m_ref);
        if (!r) delete this;
        return r;
    }
    HRESULT STDMETHODCALLTYPE Invoke(HRESULT errorCode, ICoreWebView2Environment* env) override;
};

class CtlCompletedHandler : public ICoreWebView2CreateCoreWebView2ControllerCompletedHandler {
    LONG m_ref;
public:
    CtlCompletedHandler() : m_ref(1) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        if (riid == IID_IUnknown ||
            riid == IID_ICoreWebView2CreateCoreWebView2ControllerCompletedHandler) {
            *ppv = static_cast<ICoreWebView2CreateCoreWebView2ControllerCompletedHandler*>(this);
            AddRef(); return S_OK;
        }
        *ppv = nullptr; return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override  { return InterlockedIncrement(&m_ref); }
    ULONG STDMETHODCALLTYPE Release() override {
        ULONG r = InterlockedDecrement(&m_ref);
        if (!r) delete this;
        return r;
    }
    HRESULT STDMETHODCALLTYPE Invoke(HRESULT errorCode, ICoreWebView2Controller* ctl) override;
};

class WebMsgHandler : public ICoreWebView2WebMessageReceivedEventHandler {
    LONG m_ref;
public:
    WebMsgHandler() : m_ref(1) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        if (riid == IID_IUnknown || riid == IID_ICoreWebView2WebMessageReceivedEventHandler) {
            *ppv = static_cast<ICoreWebView2WebMessageReceivedEventHandler*>(this);
            AddRef(); return S_OK;
        }
        *ppv = nullptr; return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override  { return InterlockedIncrement(&m_ref); }
    ULONG STDMETHODCALLTYPE Release() override {
        ULONG r = InterlockedDecrement(&m_ref);
        if (!r) delete this;
        return r;
    }
    HRESULT STDMETHODCALLTYPE Invoke(ICoreWebView2* sender,
                                     ICoreWebView2WebMessageReceivedEventArgs* args) override;
};

class NavCompletedHandler : public ICoreWebView2NavigationCompletedEventHandler {
    LONG m_ref;
public:
    NavCompletedHandler() : m_ref(1) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        if (riid == IID_IUnknown || riid == IID_ICoreWebView2NavigationCompletedEventHandler) {
            *ppv = static_cast<ICoreWebView2NavigationCompletedEventHandler*>(this);
            AddRef(); return S_OK;
        }
        *ppv = nullptr; return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override  { return InterlockedIncrement(&m_ref); }
    ULONG STDMETHODCALLTYPE Release() override {
        ULONG r = InterlockedDecrement(&m_ref);
        if (!r) delete this;
        return r;
    }
    HRESULT STDMETHODCALLTYPE Invoke(ICoreWebView2* sender,
                                     ICoreWebView2NavigationCompletedEventArgs* args) override {
        (void)sender; (void)args;
        LockCS(); g_navReady = true; UnlockCS();
        return S_OK;
    }
};

// ---------------------------------------------------------------------------
// 子类化窗口过程（运行在 Inno 主线程）+ 动作泵
// ---------------------------------------------------------------------------

static LRESULT CALLBACK BridgeWndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    if (msg == WM_TIMER && wp == TIMER_PUMP) {
        bool has = false;
        LockCS(); has = (g_pending != ACTION_NONE); UnlockCS();
        if (has) {
            void* thunk = g_thunk; // 主线程读取；RegisterHandler 也在主线程
            if (thunk) {
                typedef int (__stdcall *ThunkFn)();
                ((ThunkFn)thunk)();
            }
        }
        return 0; // 不再转发该定时器
    }
    if (g_origProc)
        return CallWindowProcW((WNDPROC)g_origProc, hwnd, msg, wp, lp);
    return DefWindowProcW(hwnd, msg, wp, lp);
}

static int MapAction(const std::wstring& action)
{
    if (action == L"derive")      return ACTION_DERIVE;
    if (action == L"install")     return ACTION_INSTALL;
    if (action == L"chooseDir")   return ACTION_CHOOSE_DIR;
    if (action == L"finish")      return ACTION_FINISH;
    if (action == L"confirmExit") return ACTION_CONFIRMEXIT;
    return ACTION_NONE;
}

// ---------------------------------------------------------------------------
// COM 回调实现
// ---------------------------------------------------------------------------

HRESULT STDMETHODCALLTYPE EnvCompletedHandler::Invoke(HRESULT errorCode, ICoreWebView2Environment* env)
{
    if (FAILED(errorCode) || !env || !g_parent) return S_OK; // 保持未就绪
    env->AddRef();
    g_env = env;
    env->CreateCoreWebView2Controller(g_parent, new CtlCompletedHandler());
    return S_OK;
}

HRESULT STDMETHODCALLTYPE CtlCompletedHandler::Invoke(HRESULT errorCode, ICoreWebView2Controller* ctl)
{
    if (FAILED(errorCode) || !ctl || !g_parent) return S_OK;

    g_controller = ctl;
    ICoreWebView2* wv = nullptr;
    if (SUCCEEDED(ctl->get_CoreWebView2(&wv)) && wv) {
        g_webview = wv;
        EventRegistrationToken t1, t2;
        wv->add_WebMessageReceived(new WebMsgHandler(), &t1);
        wv->add_NavigationCompleted(new NavCompletedHandler(), &t2);

        RECT rc;
        if (GetClientRect(g_parent, &rc))
            ctl->put_Bounds(rc);

        std::wstring url;
        LockCS(); url = g_url; UnlockCS();
        if (!url.empty()) wv->Navigate(url.c_str());
    } else if (wv) {
        wv->Release();
    }

    // 主线程泵：定时器 + 子类化（WM_TIMER 派发到窗口所属线程即 Inno 主线程）
    SetTimer(g_parent, TIMER_PUMP, 40, nullptr);
    g_origProc = SetWindowLongPtrW(g_parent, GWLP_WNDPROC, (LONG_PTR)BridgeWndProc);

    LockCS(); g_ctlReady = true; UnlockCS();
    return S_OK;
}

HRESULT STDMETHODCALLTYPE WebMsgHandler::Invoke(ICoreWebView2* sender,
                                                ICoreWebView2WebMessageReceivedEventArgs* args)
{
    (void)sender;
    if (!args) return S_OK;

    LPWSTR json = nullptr;
    if (FAILED(args->get_WebMessageAsJson(&json)) || !json) return S_OK;
    std::wstring text = json;
    CoTaskMemFree(json);

    std::wstring action;
    JsonGetString(text, L"action", action);
    int type = MapAction(action);

    LockCS();
    static const wchar_t* kKeys[] = { L"dir", L"desktop", L"admin", L"run", L"openCfg" };
    for (const wchar_t* k : kKeys) {
        std::wstring v;
        if (JsonGetString(text, k, v)) g_fields[k] = v;
    }
    if (type == ACTION_DERIVE) {
        // 桥自己计算派生结果，Pascal 只需 WV2_Field(L"deriveJson") + WV2_Post 转发
        std::wstring dir;
        std::map<std::wstring, std::wstring>::iterator it = g_fields.find(L"dir");
        if (it != g_fields.end()) dir = it->second;
        g_fields[L"deriveJson"] = AsciiToWide(BuildDeriveJson(dir));
    }
    g_pending = type;
    UnlockCS();
    return S_OK;
}

// ---------------------------------------------------------------------------
// 桥线程
// ---------------------------------------------------------------------------

static DWORD WINAPI BridgeThreadProc(LPVOID param)
{
    (void)param;
    HRESULT hr = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    bool comOk = SUCCEEDED(hr);

    wchar_t lad[MAX_PATH];
    std::wstring userData;
    if (SUCCEEDED(SHGetFolderPathW(nullptr, CSIDL_LOCAL_APPDATA, nullptr, SHGFP_TYPE_CURRENT, lad)))
        userData = PathJoin(lad, L"JwInstallerWV2");
    else
        userData = L"JwInstallerWV2";

    hr = CreateCoreWebView2EnvironmentWithOptions(nullptr, userData.c_str(), nullptr,
                                                  new EnvCompletedHandler());
    if (FAILED(hr)) {
        // 运行时缺失等：保持未就绪，WV2_Ready 一直返回 0，Inno 侧可兜底
    }

    MSG msg;
    while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
        if (msg.message == WM_APP_POST) {
            BSTR b = (BSTR)msg.lParam;
            if (b) {
                ICoreWebView2* wv = g_webview; // 本线程 STA 拥有该对象
                if (wv) wv->PostWebMessageAsJson(b);
                SysFreeString(b);
            }
        } else if (msg.message == WM_APP_RESIZE) {
            if (g_controller) {
                RECT rc;
                rc.left = 0; rc.top = 0;
                rc.right = (LONG)msg.wParam;
                rc.bottom = (LONG)msg.lParam;
                g_controller->put_Bounds(rc);
            }
        } else {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    if (g_webview)    { g_webview->Release();    g_webview = nullptr; }
    if (g_controller) { g_controller->Close();   g_controller->Release(); g_controller = nullptr; }
    if (g_env)        { g_env->Release();        g_env = nullptr; }
    if (comOk) CoUninitialize();
    return 0;
}

// ---------------------------------------------------------------------------
// 导出函数
// ---------------------------------------------------------------------------

extern "C" {

int __stdcall WV2_Init(HWND parent, BSTR htmlPath)
{
    EnsureCS();
    if (!parent || !htmlPath || SysStringLen(htmlPath) == 0) return -1;

    std::wstring path = htmlPath;
    // 反斜杠 → 正斜杠，构造 file:/// URL
    for (size_t i = 0; i < path.size(); ++i)
        if (path[i] == L'\\') path[i] = L'/';
    std::wstring url = L"file:///" + path;

    LockCS();
    g_parent = parent;
    g_url    = url;
    g_fields.clear();
    g_pending  = ACTION_NONE;
    g_ctlReady = false;
    g_navReady = false;
    UnlockCS();

    if (g_thread) return 0; // 已启动（重复 Init 只更新 URL）

    g_thread = CreateThread(nullptr, 0, BridgeThreadProc, nullptr, 0, &g_threadId);
    return g_thread ? 0 : -1;
}

void __stdcall WV2_Resize(int w, int h)
{
    if (!g_threadId) return;
    PostThreadMessageW(g_threadId, WM_APP_RESIZE, (WPARAM)w, (LPARAM)h);
}

void __stdcall WV2_Post(BSTR json)
{
    if (!g_threadId || !json) return;
    BSTR copy = SysAllocStringLen(json, SysStringLen(json));
    if (!copy) return;
    if (!PostThreadMessageW(g_threadId, WM_APP_POST, 0, (LPARAM)copy))
        SysFreeString(copy); // 投递失败就地释放，避免泄漏
}

int __stdcall WV2_Ready()
{
    LockCS();
    int r = (g_ctlReady && g_navReady) ? 1 : 0;
    UnlockCS();
    return r;
}

void __stdcall WV2_Destroy()
{
    if (!g_csInited) return;

    // 尽力恢复窗口状态（Destroy 在主线程调用）
    if (g_origProc && g_parent) {
        SetWindowLongPtrW(g_parent, GWLP_WNDPROC, g_origProc);
        g_origProc = 0;
    }
    if (g_parent) KillTimer(g_parent, TIMER_PUMP);

    if (g_threadId) PostThreadMessageW(g_threadId, WM_QUIT, 0, 0);
    if (g_thread) {
        WaitForSingleObject(g_thread, 3000);
        CloseHandle(g_thread);
        g_thread   = nullptr;
        g_threadId = 0;
    }

    LockCS();
    g_ctlReady = false;
    g_navReady = false;
    g_pending  = ACTION_NONE;
    g_fields.clear();
    UnlockCS();
    g_thunk = nullptr;
}

BSTR __stdcall WV2_Browse(HWND parent, BSTR title)
{
    bool comInit = false;
    HRESULT ci = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    if (SUCCEEDED(ci)) comInit = true; // 已处于其他模式时直接使用现有 apartment

    BSTR result = nullptr;
    IFileDialog* dlg = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_FileOpenDialog, nullptr, CLSCTX_INPROC_SERVER,
                                  IID_IFileDialog, (void**)&dlg);
    if (SUCCEEDED(hr) && dlg) {
        DWORD opts = 0;
        dlg->GetOptions(&opts);
        dlg->SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM);
        dlg->SetTitle(title ? title : L"");
        if (SUCCEEDED(dlg->Show(parent ? parent : GetActiveWindow()))) {
            IShellItem* item = nullptr;
            if (SUCCEEDED(dlg->GetResult(&item)) && item) {
                LPWSTR path = nullptr;
                if (SUCCEEDED(item->GetDisplayName(SIGDN_FILESYSPATH, &path)) && path) {
                    result = SysAllocString(path);
                    CoTaskMemFree(path);
                }
                item->Release();
            }
        }
        dlg->Release();
    }
    if (comInit) CoUninitialize();
    return result; // 取消/失败返回 NULL
}

int __stdcall WV2_PendingAction()
{
    LockCS();
    int p = g_pending;
    g_pending = ACTION_NONE;
    UnlockCS();
    return p;
}

BSTR __stdcall WV2_Field(BSTR key)
{
    if (!key) return SysAllocString(L"");
    std::wstring v;
    LockCS();
    std::map<std::wstring, std::wstring>::iterator it = g_fields.find(key);
    if (it != g_fields.end()) v = it->second;
    UnlockCS();
    return SysAllocStringLen(v.c_str(), (UINT)v.size());
}

BSTR __stdcall WV2_Derive(BSTR dir)
{
    std::string j = BuildDeriveJson(dir ? (const wchar_t*)dir : L"");
    std::wstring w = AsciiToWide(j);
    return SysAllocStringLen(w.c_str(), (UINT)w.size());
}

void __stdcall WV2_RegisterHandler(void* thunk)
{
    if (!g_csInited) EnsureCS();
    LockCS();
    g_thunk = thunk;
    UnlockCS();
}

void* __stdcall BRIDGE_MakeThunk(void* proc, int nargs)
{
    if (!proc) return nullptr;
    unsigned char code[32];
    int len = 0;
    unsigned int addr = (unsigned int)(UINT_PTR)proc;
    unsigned char imm[4] = {
        (unsigned char)(addr & 0xFF),
        (unsigned char)((addr >> 8) & 0xFF),
        (unsigned char)((addr >> 16) & 0xFF),
        (unsigned char)((addr >> 24) & 0xFF)
    };

    code[len++] = 0xB8;                 // mov eax, proc
    for (int i = 0; i < 4; ++i) code[len++] = imm[i];

    if (nargs == 1) {
        // mov eax,[esp+4]; call eax; ret 4   → Borland register: 第 1 参数入 eax
        const unsigned char seq[] = { 0x8B,0x44,0x24,0x04, 0xFF,0xD0, 0xC2,0x04,0x00 };
        for (size_t i = 0; i < sizeof(seq); ++i) code[len++] = seq[i];
    } else if (nargs == 2) {
        // mov edx,[esp+4]; mov eax,[esp+8]; call eax; ret 8
        const unsigned char seq[] = { 0x8B,0x54,0x24,0x04, 0x8B,0x44,0x24,0x08,
                                      0xFF,0xD0, 0xC2,0x08,0x00 };
        for (size_t i = 0; i < sizeof(seq); ++i) code[len++] = seq[i];
    } else {
        // nargs == 0：call eax; ret
        code[len++] = 0xFF;
        code[len++] = 0xD0;
        code[len++] = 0xC3;
    }

    void* mem = VirtualAlloc(nullptr, 64, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE);
    if (!mem) return nullptr;
    memcpy(mem, code, (size_t)len);
    FlushInstructionCache(GetCurrentProcess(), mem, (size_t)len);
    return mem;
}

} // extern "C"
