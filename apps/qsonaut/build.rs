#[cfg(target_os = "windows")]
fn main() {
    use image::imageops::FilterType;
    use std::path::PathBuf;

    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let png_path = manifest_dir
        .join("..")
        .join("..")
        .join("assets")
        .join("branding")
        .join("qsonaut-icon.png");
    println!("cargo:rerun-if-changed={}", png_path.display());

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let ico_path = out_dir.join("qsonaut-icon.ico");

    let image = image::open(&png_path)
        .expect("failed to load qsonaut icon PNG for Windows resource embedding");
    let icon = image.resize(256, 256, FilterType::Lanczos3);
    icon.save_with_format(&ico_path, image::ImageFormat::Ico)
        .expect("failed to generate qsonaut ICO for Windows resource embedding");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(ico_path.to_str().expect("ICO path contains invalid UTF-8"));
    resource
        .compile()
        .expect("failed to compile Windows icon resources");
}

#[cfg(not(target_os = "windows"))]
fn main() {
    configure_rade_runtime_path();
}

#[cfg(target_os = "windows")]
fn configure_rade_runtime_path() {}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn configure_rade_runtime_path() {
    use std::path::PathBuf;

    println!("cargo:rerun-if-env-changed=RADE_C_LIB_DIR");
    println!("cargo:rerun-if-env-changed=RADE_CACHE_ROOT");
    let lib_dir = std::env::var_os("RADE_C_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
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
        });
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
}
