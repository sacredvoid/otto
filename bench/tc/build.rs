use std::{env, fs, path::{Path, PathBuf}};

fn main() {
    let Some(runtime_dir) = env::var_os("DEP_TRANSCRIBE_CPP_RUNTIME_DIR") else {
        return;
    };
    // Stage next to the built exe: <target>/<profile>/
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    // OUT_DIR = <target>/<profile>/build/<pkg>-<hash>/out
    let profile_dir = out_dir.ancestors().nth(3).unwrap().to_path_buf();
    let dest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("transcribe-libs");
    fs::create_dir_all(&dest).unwrap();

    fn copy_libs(src: &Path, dests: &[&Path]) -> usize {
        let mut n = 0;
        for entry in fs::read_dir(src).unwrap_or_else(|e| panic!("read {}: {e}", src.display())) {
            let path = entry.unwrap().path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let is_lib = name.ends_with(".dll") || name.ends_with(".dylib") || name.ends_with(".so") || name.contains(".so.");
            if is_lib {
                for d in dests {
                    fs::copy(&path, d.join(name)).unwrap();
                }
                n += 1;
            }
        }
        n
    }

    let runtime_dir = PathBuf::from(runtime_dir);
    assert!(copy_libs(&runtime_dir, &[&dest, &profile_dir]) > 0, "no runtime libraries in {}", runtime_dir.display());
    println!("cargo:warning=RUNTIME_DIR={}", runtime_dir.display());
    println!("cargo:rerun-if-env-changed=DEP_TRANSCRIBE_CPP_RUNTIME_DIR");
    println!("cargo:rerun-if-env-changed=DEP_TRANSCRIBE_CPP_MODULE_DIR");
    if let Some(module_dir) = env::var_os("DEP_TRANSCRIBE_CPP_MODULE_DIR") {
        let module_dir = PathBuf::from(module_dir);
        println!("cargo:warning=MODULE_DIR={}", module_dir.display());
        assert!(copy_libs(&module_dir, &[&dest, &profile_dir]) > 0, "no backend modules in {}", module_dir.display());
    }
}
