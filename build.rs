use std::{env, fs, io, path::Path};

fn assets(root: &Path, dir: &Path, code: &mut String) -> io::Result<()> {
    if dir.exists() {
        let mut entries = fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                assets(root, &path, code)?;
            } else {
                let route = path.strip_prefix(root).unwrap().to_string_lossy();
                let mime = match path.extension().and_then(|v| v.to_str()) {
                    Some("html") => "text/html; charset=utf-8",
                    Some("js") => "text/javascript",
                    Some("css") => "text/css",
                    Some("svg") => "image/svg+xml",
                    _ => "application/octet-stream",
                };
                code.push_str(&format!(
                    "{route:?} => Some(({mime:?},include_bytes!({:?}))),\n",
                    path.canonicalize()?
                ));
            }
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    // Keep compiled example evaluators within the 4 MiB driver limit in debug builds.
    if matches!(
        env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("linux" | "macos")
    ) {
        println!("cargo::rustc-link-arg-examples=-Wl,-S");
    }
    let root = Path::new("web/dist");
    println!("cargo:rerun-if-changed=web/dist");
    let mut code = String::from(
        "pub fn asset(path:&str)->Option<(&'static str,&'static [u8])>{match path {\n",
    );
    assets(root, root, &mut code)?;
    code.push_str("_ => None }}");
    fs::write(
        Path::new(&env::var_os("OUT_DIR").unwrap()).join("assets.rs"),
        code,
    )
}
