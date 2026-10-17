//! 单实例控制：命名互斥体 + 激活已有窗口
//!
//! 启动时尝试创建 `Global\JiweiToolboxSingleInstance` 互斥体；
//! 若已存在（ERROR_ALREADY_EXISTS）说明已有实例在运行，
//! 通过 FindWindowW 找到标题为"极维工具箱"的窗口，
//! ShowWindow + SetForegroundWindow 激活后退出当前进程。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

const MUTEX_NAME: &str = "Global\\JiweiToolboxSingleInstance";
const WINDOW_TITLE: &str = "极维工具箱";

/// UTF-16 宽字符串（含结尾 null）
fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// 尝试获取单实例互斥体。
/// 若已有实例运行：激活其主窗口并 `process::exit(0)`；
/// 否则持有句柄（故意泄漏，进程生命周期内保持互斥体有效）并返回。
pub fn acquire_or_exit() {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    unsafe {
        let handle: HANDLE = CreateMutexW(std::ptr::null(), 0, to_wide(MUTEX_NAME).as_ptr());
        if handle.is_null() {
            // 创建互斥体失败（极少见）：不阻断启动
            return;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // 已有实例：找到其主窗口并激活
            let title = to_wide(WINDOW_TITLE);
            let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
            if !hwnd.is_null() {
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
            CloseHandle(handle);
            std::process::exit(0);
        }
        // 故意不关闭句柄：裸指针离开作用域不会触发 CloseHandle，
        // 句柄保持打开直至进程退出，期间互斥体持续有效
        let _ = handle;
    }
}
