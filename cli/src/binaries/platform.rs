use std::env;
use std::ops::Add;

/// Engine assets depend on the Rust target, not the host OpenSSL installation.
pub fn binary_platform_name() -> Result<&'static str, String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("linux", "aarch64") if cfg!(target_env = "gnu") => Ok("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64") if cfg!(target_env = "gnu") => Ok("x86_64-unknown-linux-gnu"),
        (os, arch) => Err(format!(
            "No Gitar rustls engines are published for {os}/{arch}"
        )),
    }
}

pub fn arch() -> String {
    match env::consts::ARCH {
        "x86_64" => "x64".to_string(),
        "aarch64" => "arm64".to_string(),
        arch => panic!("Architecture {arch} is not yet supported"),
    }
}

pub fn name() -> String {
    match env::consts::OS {
        "macos" => "darwin".to_string(),
        os => os.to_string(),
    }
}

pub fn check_for_extension(platform: &str, path: &str) -> String {
    let path = path.to_string();

    if platform == "windows" {
        if path.contains(".gz") {
            return path.replace(".gz", ".exe.gz");
        }
        return path.add(".exe");
    }

    path
}
