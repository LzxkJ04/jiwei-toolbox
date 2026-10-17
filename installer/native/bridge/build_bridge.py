# -*- coding: utf-8 -*-
"""编译 32 位 WebView2 桥 DLL（wv2bridge.dll）。

用 python subprocess 以参数列表方式调用 MSVC（避免 .bat 的 CRLF/转义问题与
git bash 的 MSYS 路径转换问题）。可重复执行。
"""
import os
import shutil
import subprocess
import sys

ROOT = r"C:\Users\Administrator\Documents\极维工具箱\installer"
BRIDGE_DIR = os.path.join(ROOT, "native", "bridge")
WV2_INC = os.path.join(ROOT, "native", "wv2", "sdk", "build", "native", "include")
WV2_X86 = os.path.join(ROOT, "native", "wv2", "sdk", "build", "native", "x86")

MSVC_DIR = r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.44.35207"
WINSDK_VER = "10.0.26100.0"
WINSDK_INC = r"C:\Program Files (x86)\Windows Kits\10\Include" + "\\" + WINSDK_VER
WINSDK_LIB = r"C:\Program Files (x86)\Windows Kits\10\Lib" + "\\" + WINSDK_VER

CL = os.path.join(MSVC_DIR, "bin", "Hostx64", "x86", "cl.exe")


def ensure_utf8_bom(path):
    """源文件必须是 UTF-8 带 BOM，否则 MSVC 按 GBK 误读中文字符串。"""
    with open(path, "rb") as f:
        data = f.read()
    if not data.startswith(b"\xef\xbb\xbf"):
        with open(path, "wb") as f:
            f.write(b"\xef\xbb\xbf" + data)
        print("[build] 已为 %s 添加 UTF-8 BOM" % path)
    else:
        print("[build] UTF-8 BOM 已存在")


def main():
    src = os.path.join(BRIDGE_DIR, "wv2bridge.cpp")
    ensure_utf8_bom(src)

    include = ";".join([
        WV2_INC,
        os.path.join(MSVC_DIR, "include"),
        os.path.join(WINSDK_INC, "ucrt"),
        os.path.join(WINSDK_INC, "um"),
        os.path.join(WINSDK_INC, "shared"),
        os.path.join(WINSDK_INC, "winrt"),
        os.path.join(WINSDK_INC, "cppwinrt"),
    ])
    lib = ";".join([
        os.path.join(MSVC_DIR, "lib", "x86"),
        os.path.join(WINSDK_LIB, "ucrt", "x86"),
        os.path.join(WINSDK_LIB, "um", "x86"),
    ])

    cmd = [
        CL,
        "/nologo", "/O2", "/MD", "/EHsc",
        "/D_WINDOWS", "/D_UNICODE", "/DUNICODE",
        "/std:c++17", "/D_HAS_STD_BYTE=0",
        "/I" + WV2_INC,
        "wv2bridge.cpp",
        "/link",
        "/DLL", "/OUT:wv2bridge.dll",
        "/DEF:wv2bridge.def",
        "/LIBPATH:" + WV2_X86, "WebView2LoaderStatic.lib",
        "user32.lib", "gdi32.lib", "shell32.lib", "advapi32.lib",
        "ole32.lib", "oleaut32.lib", "uuid.lib", "shlwapi.lib",
        "/MACHINE:X86", "/SUBSYSTEM:WINDOWS",
    ]

    env = os.environ.copy()
    env["INCLUDE"] = include
    env["LIB"] = lib

    print("[build] " + " ".join(cmd))
    r = subprocess.run(cmd, cwd=BRIDGE_DIR, env=env,
                       capture_output=True, text=True, errors="replace")
    if r.stdout:
        print(r.stdout)
    if r.stderr:
        print(r.stderr, file=sys.stderr)
    if r.returncode != 0:
        print("[build] 编译失败，退出码 %d" % r.returncode, file=sys.stderr)
        return 1

    dll = os.path.join(BRIDGE_DIR, "wv2bridge.dll")
    out = os.path.join(ROOT, "res", "bridge", "wv2bridge.dll")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    shutil.copy2(dll, out)
    print("[build] 产物: %s (%d bytes)" % (out, os.path.getsize(out)))
    print("[build] 副本: %s (%d bytes)" % (dll, os.path.getsize(dll)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
