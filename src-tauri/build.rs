fn main() {
    #[cfg(target_os = "linux")]
    {
        if let Ok(out_dir) = std::env::var("OUT_DIR") {
            let out_path = std::path::Path::new(&out_dir);
            let link_dest = out_path.join("libxdo.so");
            if !link_dest.exists() {
                for candidate in &[
                    "/usr/lib/x86_64-linux-gnu/libxdo.so.3",
                    "/usr/lib/aarch64-linux-gnu/libxdo.so.3",
                    "/usr/lib64/libxdo.so.3",
                    "/usr/lib/libxdo.so.3",
                ] {
                    if std::path::Path::new(candidate).exists() {
                        let _ = std::os::unix::fs::symlink(candidate, &link_dest);
                        println!("cargo:rustc-link-search=native={}", out_dir);
                        break;
                    }
                }
            } else {
                println!("cargo:rustc-link-search=native={}", out_dir);
            }
        }
    }

    tauri_build::build()
}
