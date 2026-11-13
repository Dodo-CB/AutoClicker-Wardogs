fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Wardogs Autoklicker");
        res.set("FileDescription", "Wardogs Autoklicker");
        res.compile().expect("Icon konnte nicht eingebettet werden");
    }
}