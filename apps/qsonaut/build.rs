#[cfg(target_os = "windows")]
fn main() {
    use std::path::PathBuf;
    use image::imageops::FilterType;

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
    icon
        .save_with_format(&ico_path, image::ImageFormat::Ico)
        .expect("failed to generate qsonaut ICO for Windows resource embedding");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(ico_path.to_str().expect("ICO path contains invalid UTF-8"));
    resource
        .compile()
        .expect("failed to compile Windows icon resources");
}

#[cfg(not(target_os = "windows"))]
fn main() {}
