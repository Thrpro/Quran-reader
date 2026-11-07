fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        use std::{env, fs, path::PathBuf, process::Command};
        println!("cargo:rerun-if-changed=assets/app.ico");
        println!("cargo:rerun-if-changed=app.manifest");
        println!("cargo:rerun-if-env-changed=RC");
        let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let quoted = |name: &str| root.join(name).to_string_lossy().replace('\\', "\\\\");
        let rc = out.join("reader.rc");
        fs::write(
            &rc,
            format!(
                "1 ICON \"{}\"\n1 24 \"{}\"\n",
                quoted("assets/app.ico"),
                quoted("app.manifest")
            ),
        )
        .unwrap();
        let compiler = env::var_os("RC").unwrap_or_else(|| {
            "C:/Program Files (x86)/Windows Kits/10/bin/10.0.26100.0/x64/rc.exe".into()
        });
        let resource = out.join("reader.res");
        assert!(
            Command::new(compiler)
                .arg("/nologo")
                .arg("/fo")
                .arg(&resource)
                .arg(&rc)
                .status()
                .expect("Windows SDK resource compiler (set RC)")
                .success()
        );
        println!("cargo:rustc-link-arg={}", resource.display());
    }
    slint_build::compile_with_config(
        "ui/reader.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .expect("Compile reading prototype UI");
}
