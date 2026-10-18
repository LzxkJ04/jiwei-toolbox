fn main() {
    // 图标文件变更时必须重新跑构建脚本，否则会沿用旧的 resource.rc。
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    tauri_build::build()
}
