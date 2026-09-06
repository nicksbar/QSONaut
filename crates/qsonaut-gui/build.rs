use std::path::PathBuf;

fn main() {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        println!("cargo:rerun-if-env-changed=RADE_C_LIB_DIR");
        println!("cargo:rerun-if-env-changed=RADE_CACHE_ROOT");
        let lib_dir = std::env::var_os("RADE_C_LIB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(default_rade_lib_dir);
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn default_rade_lib_dir() -> PathBuf {
    let cache_root = std::env::var_os("RADE_CACHE_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .map(|path| path.join("qsonaut-third-party"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".cache").join("qsonaut-third-party"))
        })
        .unwrap_or_else(|| PathBuf::from("target/native"));
    cache_root
        .join("rade_c")
        .join("0d5c5f7c27e650e3ca8d9e0f7d4781f2e73ee2b0")
        .join("build")
        .join("src")
}
