fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Compresor de vídeo");
        res.set("FileDescription", "Compresor de vídeo");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no se pudo incrustar el icono en el .exe: {e}");
        }
    }
}
