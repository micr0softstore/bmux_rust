fn main() {
    println!("cargo:rerun-if-changed=assets/bmux.ico");
    println!("cargo:rerun-if-changed=packaging/bmux.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/bmux.ico")
            .set("ProductName", "[bmux]")
            .set("FileDescription", "Bmux workspace terminal")
            .set("OriginalFilename", "bmux.exe")
            .set_manifest_file("packaging/bmux.manifest")
            .compile()
            .expect("could not compile the bmux icon/manifest; install Windows SDK (MSVC) or MinGW binutils (GNU)");
    }
}
