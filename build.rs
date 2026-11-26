use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=app.ico");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=payload.txt");
    embed_resource::compile("app.rc", embed_resource::NONE).manifest_required().unwrap();

    let root = env::var("CARGO_MANIFEST_DIR").unwrap();
    let list = fs::read_to_string(Path::new(&root).join("payload.txt")).expect("payload.txt");
    let names: Vec<&str> = list.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.split('=').next().unwrap().trim()).collect();
    let mut code = format!("pub(crate) const PAYLOAD: [(&str, &[u8]); {}] = [\n", names.len());
    for n in &names {
        let p = Path::new(&root).join("payload").join(n);
        assert!(p.is_file(), r"payload\{n} is missing: run scripts\build-all.ps1 (or scripts\build-plugins.ps1) first");
        println!("cargo:rerun-if-changed={}", p.display());
        code += &format!("    ({n:?}, include_bytes!({:?})),\n", p.display().to_string());
    }
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("payload.rs"), code + "];\n").unwrap();
}
