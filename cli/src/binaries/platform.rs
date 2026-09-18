use std::env;

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
