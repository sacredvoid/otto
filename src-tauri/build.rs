use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    // Metal code in the speech engine uses @available checks. With a deployment target below
    // the build machine's OS (set by bundle.macOS.minimumSystemVersion) those become calls into
    // clang's runtime (___isPlatformVersionAtLeast), which rustc doesn't link by default.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let dir = std::process::Command::new("clang").arg("--print-runtime-dir").output().ok();
        if let Some(dir) = dir.filter(|o| o.status.success()) {
            println!("cargo:rustc-link-search=native={}", String::from_utf8_lossy(&dir.stdout).trim());
            println!("cargo:rustc-link-lib=static=clang_rt.osx");
        }
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        stage_windows_libs();
    }
    tauri_build::build()
}

/// Windows (dynamic-backends): transcribe.dll, ggml*.dll and the backend modules (Vulkan,
/// per-ISA CPU) are separate DLLs. Copy them into `transcribe-libs/` (bundled next to
/// Otto.exe by tauri.windows.conf.json) and next to the built exe/tests so `cargo run`,
/// `cargo test` and `tauri dev` find them. Plus the VC++ runtime, app-local, for PCs
/// without the redistributable installed.
fn stage_windows_libs() {
    println!("cargo:rerun-if-env-changed=DEP_TRANSCRIBE_CPP_RUNTIME_DIR");
    println!("cargo:rerun-if-env-changed=DEP_TRANSCRIBE_CPP_MODULE_DIR");
    println!("cargo:rerun-if-env-changed=VCToolsRedistDir");
    // Static build (no dynamic-backends): nothing to ship.
    let Some(runtime_dir) = env::var_os("DEP_TRANSCRIBE_CPP_RUNTIME_DIR") else {
        return;
    };
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    // OUT_DIR = <target>/<profile>/build/<pkg>-<hash>/out
    let profile_dir = out_dir.ancestors().nth(3).unwrap().to_path_buf();
    let deps_dir = profile_dir.join("deps"); // test binaries run from here
    let dest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("transcribe-libs");
    for d in [&dest, &deps_dir] {
        fs::create_dir_all(d).unwrap_or_else(|e| panic!("create {}: {e}", d.display()));
    }
    let dests: [&Path; 3] = [&dest, &profile_dir, &deps_dir];
    // Re-stage when a staged copy is deleted (git clean, fresh clone) while the out-of-tree
    // target dir survives: a missing path counts as changed, and the folder's mtime covers
    // files removed from it.
    println!("cargo:rerun-if-changed={}", dest.display());
    for d in [&dest, &profile_dir, &deps_dir] {
        println!("cargo:rerun-if-changed={}", d.join("transcribe.dll").display());
    }

    let runtime_dir = PathBuf::from(runtime_dir);
    let n = copy_dlls(&runtime_dir, &dests, |_| true);
    assert!(n > 0, "no transcribe-cpp runtime DLLs in {}", runtime_dir.display());
    if let Some(module_dir) = env::var_os("DEP_TRANSCRIBE_CPP_MODULE_DIR").map(PathBuf::from) {
        if module_dir != runtime_dir {
            let n = copy_dlls(&module_dir, &dests, |_| true);
            assert!(n > 0, "no ggml backend modules in {}", module_dir.display());
        }
    }

    // transcribe.dll and ggml*.dll import these three; without them Otto can't start on a PC
    // lacking the VC++ redistributable. OpenMP is off, so nothing imports vcomp140.
    const REQUIRED: [&str; 3] = ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"];
    let release = env::var("PROFILE").as_deref() == Ok("release");
    match vc_redist_dir() {
        Some(redist) => {
            let wanted = |n: &str| REQUIRED.contains(&n.to_ascii_lowercase().as_str());
            for sub in fs::read_dir(&redist).into_iter().flatten().flatten() {
                // Microsoft.VC143.CRT
                let p = sub.path();
                if p.is_dir() {
                    copy_dlls(&p, &[&dest], wanted);
                }
            }
            for n in REQUIRED {
                if dest.join(n).is_file() {
                    continue;
                }
                let msg = format!("{n} not found in {}", redist.display());
                if release {
                    panic!("{msg}; the installer would not start without the VC++ redistributable");
                }
                println!("cargo:warning={msg}");
            }
        }
        None => {
            let msg = "VC++ redist folder not found; the installer will not carry msvcp140/vcruntime140";
            if release {
                panic!("{msg} (set VCToolsRedistDir)");
            }
            println!("cargo:warning={msg}");
        }
    }
}

fn copy_dlls(src: &Path, dests: &[&Path], keep: impl Fn(&str) -> bool) -> usize {
    let mut n = 0;
    let entries = fs::read_dir(src).unwrap_or_else(|e| panic!("read {}: {e}", src.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if !name.to_ascii_lowercase().ends_with(".dll") || !keep(name) {
            continue;
        }
        for d in dests {
            let to = d.join(name);
            // Skip identical files so a running exe holding the DLL doesn't fail the build.
            let same = fs::metadata(&to).ok().zip(entry.metadata().ok()).is_some_and(|(a, b)| {
                a.len() == b.len() && a.modified().ok() >= b.modified().ok()
            });
            if !same {
                fs::copy(&path, &to).unwrap_or_else(|e| panic!("copy {} -> {}: {e}", path.display(), to.display()));
            }
        }
        n += 1;
    }
    n
}

/// `<VS>\VC\Redist\MSVC\<version>\<arch>`: from a VS developer shell's `VCToolsRedistDir`,
/// else the newest Visual Studio found by vswhere, else the newest under the default roots.
fn vc_redist_dir() -> Option<PathBuf> {
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86") => "x86",
        _ => "x64",
    };
    if let Some(d) = env::var_os("VCToolsRedistDir").map(|d| PathBuf::from(d).join(arch)) {
        if d.is_dir() {
            return Some(d);
        }
    }
    let mut installs: Vec<PathBuf> = Vec::new();
    let pf86 = env::var_os("ProgramFiles(x86)").map(PathBuf::from);
    if let Some(vswhere) = pf86.as_ref().map(|p| p.join("Microsoft Visual Studio/Installer/vswhere.exe")) {
        let out = std::process::Command::new(vswhere)
            .args(["-latest", "-products", "*", "-property", "installationPath"])
            .output();
        if let Some(o) = out.ok().filter(|o| o.status.success()) {
            installs.extend(String::from_utf8_lossy(&o.stdout).lines().map(|l| PathBuf::from(l.trim())));
        }
    }
    for root in [env::var_os("ProgramFiles").map(PathBuf::from), pf86].into_iter().flatten() {
        let vs = root.join("Microsoft Visual Studio");
        for year in fs::read_dir(&vs).into_iter().flatten().flatten() {
            for edition in fs::read_dir(year.path()).into_iter().flatten().flatten() {
                installs.push(edition.path());
            }
        }
    }
    for install in installs {
        let msvc = install.join("VC/Redist/MSVC");
        let mut versions: Vec<PathBuf> = fs::read_dir(&msvc)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join(arch))
            .filter(|p| p.is_dir())
            .collect();
        versions.sort();
        if let Some(d) = versions.pop() {
            return Some(d);
        }
    }
    None
}
