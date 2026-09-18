pub mod platform;
mod runner;

use directories::BaseDirs;
use flate2::read::GzDecoder;
use reqwest::blocking as reqwest;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{create_dir_all, File};
use std::io;
use std::io::prelude::*;
use std::path::{Path, PathBuf};

pub static PRISMA_CLI_VERSION: &str = "4.8.0";
pub static ENGINE_VERSION: &str = "v4.8.0-gitar.1";
pub static BASE_DIR_NAME: &str = "prisma/binaries";

pub struct Engine<'a> {
    pub name: &'a str,
    pub env: &'a str,
}

pub const ENGINES: [Engine; 4] = [
    Engine {
        name: "query-engine",
        env: "PRISMA_QUERY_ENGINE_BINARY",
    },
    Engine {
        name: "migration-engine",
        env: "PRISMA_MIGRATION_ENGINE_BINARY",
    },
    Engine {
        name: "introspection-engine",
        env: "PRISMA_INTROSPECTION_ENGINE_BINARY",
    },
    Engine {
        name: "prisma-fmt",
        env: "PRISMA_FMT_BINARY",
    },
];

pub fn global_cache_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("PRISMA_CLI_CACHE_DIR") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("PRISMA_CLI_CACHE_DIR must be absolute".into());
        }
        return Ok(path);
    }
    let base_dirs = BaseDirs::new().ok_or("Cannot resolve the Prisma cache directory")?;
    let cache_dir = base_dirs.cache_dir();

    Ok(cache_dir
        .join(BASE_DIR_NAME)
        .join("cli")
        .join(PRISMA_CLI_VERSION))
}

pub fn fetch_native(to_dir: &Path) -> Result<runner::Runner, String> {
    if !to_dir.is_absolute() {
        Err("to_dir must be absolute".to_string())?;
    }

    let runner = runner::install(to_dir)?;

    for e in &ENGINES {
        if std::env::var_os(e.env).is_none() {
            download_engine(e.name, to_dir)?;
        }
    }

    Ok(runner)
}

#[derive(serde::Deserialize)]
struct EngineManifest {
    release: String,
    targets: BTreeMap<String, BTreeMap<String, String>>,
}

fn download_engine(engine_name: &str, to_dir: &Path) -> Result<(), String> {
    let target = platform::binary_platform_name()?;
    let manifest: EngineManifest = serde_json::from_str(include_str!("engines.json"))
        .map_err(|error| format!("Invalid bundled engine manifest: {error}"))?;
    if manifest.release != ENGINE_VERSION {
        return Err("Engine release and bundled checksum manifest disagree".into());
    }
    let checksum = manifest
        .targets
        .get(target)
        .and_then(|engines| engines.get(engine_name))
        .ok_or_else(|| format!("No checksum for {engine_name} on {target}"))?;
    let to = to_dir
        .join(ENGINE_VERSION)
        .join(format!("prisma-{engine_name}-{target}"));
    if to.is_file() {
        return Ok(());
    }
    let url = format!(
        "https://github.com/gitarcode/prisma-engines/releases/download/{ENGINE_VERSION}/{engine_name}-{target}.gz"
    );
    download(&url, &to, Some(checksum))
}

fn decode_archive(bytes: &[u8], checksum: Option<&str>) -> Result<Vec<u8>, String> {
    if let Some(expected) = checksum {
        let actual = format!("{:x}", Sha256::digest(bytes));
        if actual != expected {
            return Err(format!(
                "Engine checksum mismatch: expected {expected}, got {actual}"
            ));
        }
    }
    let mut binary = Vec::new();
    GzDecoder::new(bytes)
        .read_to_end(&mut binary)
        .map_err(|error| format!("Invalid engine archive: {error}"))?;
    Ok(binary)
}

fn download(url: &str, to: &Path, checksum: Option<&str>) -> Result<(), String> {
    let parent = to
        .parent()
        .ok_or_else(|| "Download path has no parent".to_string())?;
    create_dir_all(parent)
        .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
    let response = reqwest::get(url)
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Cannot download {url}: {error}"))?;
    let bytes = response
        .bytes()
        .map_err(|error| format!("Cannot read {url}: {error}"))?;
    let binary = decode_archive(&bytes, checksum)?;
    // Concurrent generator processes must never execute a partially written engine.
    let temporary = to.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| -> io::Result<()> {
        let mut file = File::create(&temporary)?;
        file.write_all(&binary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&temporary, to)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("Cannot install {}: {error}", to.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};

    #[test]
    fn bundled_manifest_covers_release_targets() -> Result<(), Box<dyn std::error::Error>> {
        let manifest: EngineManifest = serde_json::from_str(include_str!("engines.json"))?;
        assert_eq!(manifest.release, ENGINE_VERSION);
        for target in [
            "aarch64-apple-darwin",
            "aarch64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
        ] {
            let engines = manifest
                .targets
                .get(target)
                .ok_or("missing release target")?;
            for engine in ENGINES {
                let checksum = engines.get(engine.name).ok_or("missing engine checksum")?;
                assert_eq!(checksum.len(), 64);
                assert!(checksum.bytes().all(|byte| byte.is_ascii_hexdigit()));
            }
        }
        Ok(())
    }

    #[test]
    fn archive_checksum_is_checked_before_decompression() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"engine fixture")?;
        let archive = encoder.finish()?;
        let checksum = format!("{:x}", Sha256::digest(&archive));
        assert_eq!(
            decode_archive(&archive, Some(&checksum))?,
            b"engine fixture"
        );
        assert!(decode_archive(&archive, Some("incorrect")).is_err());
        assert!(decode_archive(b"not gzip", None).is_err());
        Ok(())
    }
}
