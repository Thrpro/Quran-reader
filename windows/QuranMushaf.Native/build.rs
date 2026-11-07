fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=assets/app.ico");
    winresource::WindowsResource::new()
        .set_icon("assets/app.ico")
        .set_manifest_file("app.manifest")
        .set("ProductName", "Quran Mushaf Reader — Native Preview")
        .set("FileDescription", "Native Rust reader preview")
        .compile()
        .expect("Native Windows resources");
}
