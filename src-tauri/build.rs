fn main() {
    // whisper.cpp's Metal code uses @available checks. With a deployment target below the
    // build machine's OS (set by bundle.macOS.minimumSystemVersion) those become calls into
    // clang's runtime (___isPlatformVersionAtLeast), which rustc doesn't link by default.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let dir = std::process::Command::new("clang").arg("--print-runtime-dir").output().ok();
        if let Some(dir) = dir.filter(|o| o.status.success()) {
            println!("cargo:rustc-link-search=native={}", String::from_utf8_lossy(&dir.stdout).trim());
            println!("cargo:rustc-link-lib=static=clang_rt.osx");
        }
    }
    tauri_build::build()
}
