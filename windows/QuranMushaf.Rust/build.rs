fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=../QuranMushaf.Windows/Assets/app.ico");
    winresource::WindowsResource::new()
        .set_icon("../QuranMushaf.Windows/Assets/app.ico")
        .set_manifest_file("app.manifest")
        .set("ProductName", "Quran Mushaf Reader")
        .set("FileDescription", "Quran Mushaf Reader — Rust host")
        .set("OriginalFilename", "QuranMushaf.Windows.exe")
        .compile()
        .expect("Windows application resource compilation failed");
}
