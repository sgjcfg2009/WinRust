fn main() {
    for (bin, rc, req_manifest) in [
        ("debloat", "debloat.rc", false),
        ("photoview", "photoview.rc", false),
        ("purge", "purge.rc", false),
        ("reshell", "reshell.rc", false),
        ("rs-shell", "rs-shell.rc", false),
        ("rtaskmgr-win32", "rtaskmgr-win32.rc", true),
        ("rust-explorer", "rust-explorer.rc", true),
        ("rust-settings", "rust-settings.rc", false),
        ("topbar", "topbar.rc", false),
    ] {
        println!("cargo:rerun-if-changed={rc}");
        println!("cargo:rerun-if-changed={bin}.ico");
        if req_manifest {
            println!("cargo:rerun-if-changed={bin}.manifest");
            embed_resource::compile_for(rc, &[bin], embed_resource::NONE).manifest_required().unwrap();
        } else {
            embed_resource::compile_for(rc, &[bin], embed_resource::NONE).manifest_optional().unwrap();
        }
    }
}
