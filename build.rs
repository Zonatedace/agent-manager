// Windows resource embedding (icon + file version metadata).
// Icon file is optional — build succeeds without assets/icon.ico.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set("ProductName", "TODO Dashboard");
        res.set("FileDescription", "TODO Dashboard — local multi-repo TODO + agent shell");
        res.set("LegalCopyright", "Copyright (c) the TODO Dashboard contributors");
        res.set("CompanyName", "Zonatedace");
        // Optional icon
        let ico = std::path::Path::new("assets/icon.ico");
        if ico.exists() {
            res.set_icon("assets/icon.ico");
        }
        if let Err(e) = res.compile() {
            // Don't fail the whole build if winres has issues in CI-like envs
            println!("cargo:warning=winres: {e}");
        }
    }
}
