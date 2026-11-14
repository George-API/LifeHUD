fn main() {
    // The settings window uses Slint's Cupertino widget style; the HUD itself uses only custom components.
    let config = slint_build::CompilerConfiguration::new().with_style("cupertino".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint UI failed to compile");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id("assets/lifehud.ico", "1");
        res.set_manifest_file("assets/lifehud.manifest");
        res.compile().expect("failed to embed Windows resources");
    }
}
