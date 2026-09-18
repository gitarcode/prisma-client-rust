//! Verified Node and Prisma JavaScript installations. Never run npm install scripts.

use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{platform, PRISMA_CLI_VERSION};

const MANIFEST: &str = include_str!("runner.json");
const MAX_DOWNLOAD_BYTES: u64 = 128 * 1024 * 1024;

type Files = BTreeMap<String, String>;

#[derive(Deserialize)]
struct Manifest {
    node_version: String,
    prisma_version: String,
    nodes: BTreeMap<String, NodeArchive>,
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct NodeArchive {
    url: String,
    sha256: String,
    prefix: String,
    files: Files,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    files: Files,
}

pub struct Runner {
    pub node: PathBuf,
    pub script: PathBuf,
}

pub fn install(cache: &Path) -> Result<Runner, String> {
    let manifest: Manifest = serde_json::from_str(MANIFEST)
        .map_err(|error| format!("Invalid runner manifest: {error}"))?;
    if manifest.prisma_version != PRISMA_CLI_VERSION {
        return Err("Prisma CLI and runner manifest versions disagree".into());
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| format!("Cannot build runner download client: {error}"))?;
    let root = cache.join("runner");
    install_with(&root, &manifest, platform::binary_platform_name()?, |url| {
        let response = client
            .get(url)
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|error| format!("Cannot download {url}: {error}"))?;
        let mut bytes = Vec::new();
        response
            .take(MAX_DOWNLOAD_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Cannot read {url}: {error}"))?;
        if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
            return Err(format!("Runner download exceeds the size limit: {url}"));
        }
        Ok(bytes)
    })
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn check_digest(bytes: &[u8], expected: &str) -> Result<(), String> {
    if digest(bytes) != expected {
        return Err("Runner checksum mismatch".into());
    }
    Ok(())
}

fn relative_path(path: &str) -> Result<&Path, String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(format!("Invalid runner manifest path: {}", path.display()));
    }
    Ok(path)
}

fn verify_installation(directory: &Path, files: &Files) -> Result<(), String> {
    for (name, expected) in files {
        let path = directory.join(relative_path(name)?);
        let bytes = fs::read(&path).map_err(|error| {
            format!(
                "Cannot read cached runner {}: {error}. Remove {} and retry",
                path.display(),
                directory.display()
            )
        })?;
        check_digest(&bytes, expected).map_err(|error| {
            format!(
                "{error}: {}. Remove {} and retry",
                path.display(),
                directory.display()
            )
        })?;
    }
    Ok(())
}

/// Publish a whole verified directory. A concurrent install may win the rename.
fn install_directory(
    directory: &Path,
    files: &Files,
    populate: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if directory.exists() {
        return verify_installation(directory, files);
    }
    let parent = directory.parent().ok_or("Runner cache has no parent")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = tempfile::Builder::new()
        .prefix(".runner-")
        .tempdir_in(parent)
        .map_err(|error| error.to_string())?;
    populate(temporary.path())?;
    verify_installation(temporary.path(), files)?;
    match fs::rename(temporary.path(), directory) {
        Ok(()) => Ok(()),
        Err(_) if directory.exists() => verify_installation(directory, files),
        Err(error) => Err(format!("Cannot install {}: {error}", directory.display())),
    }
}

fn write_verified(
    directory: &Path,
    name: &str,
    bytes: &[u8],
    checksum: &str,
) -> Result<(), String> {
    check_digest(bytes, checksum)?;
    let path = directory.join(relative_path(name)?);
    fs::create_dir_all(path.parent().ok_or("Runner file has no parent")?)
        .map_err(|error| error.to_string())?;
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    if matches!(path.file_name(), Some(name) if name == "xdg-open") {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn unpack_node(directory: &Path, archive: &NodeArchive, bytes: &[u8]) -> Result<(), String> {
    // Authenticate the compressed bytes before parsing any archive entries.
    check_digest(bytes, &archive.sha256)?;
    let mut tar = tar::Archive::new(GzDecoder::new(bytes));
    for entry in tar.entries().map_err(|error| error.to_string())? {
        let mut entry = entry.map_err(|error| error.to_string())?;
        let path = entry
            .path()
            .map_err(|error| error.to_string())?
            .into_owned();
        let name = match path
            .strip_prefix(&archive.prefix)
            .ok()
            .and_then(Path::to_str)
        {
            Some(name) if archive.files.contains_key(name) => name,
            _ => continue,
        };
        if !entry.header().entry_type().is_file() {
            return Err(format!("Node archive entry is not a file: {name}"));
        }
        let expected = archive
            .files
            .get(name)
            .ok_or("Missing Node file checksum")?;
        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|error| error.to_string())?;
        write_verified(directory, name, &data, expected)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            directory.join("bin/node"),
            fs::Permissions::from_mode(0o755),
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn install_with(
    cache: &Path,
    manifest: &Manifest,
    target: &str,
    mut fetch: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Runner, String> {
    let node = manifest
        .nodes
        .get(target)
        .ok_or("No Node runtime for this target")?;
    let node_dir = cache.join(format!("node-{}-{}", manifest.node_version, node.sha256));
    install_directory(&node_dir, &node.files, |directory| {
        unpack_node(directory, node, &fetch(&node.url)?)
    })?;

    let mut files = Files::new();
    let mut downloads = BTreeMap::new();
    for package in &manifest.packages {
        relative_path(&package.name)?;
        let prefix = if package.name == "prisma" {
            "prisma".to_string()
        } else {
            format!("prisma/node_modules/{}", package.name)
        };
        for (name, checksum) in &package.files {
            relative_path(name)?;
            let path = format!("{prefix}/{name}");
            files.insert(path.clone(), checksum.clone());
            downloads.insert(
                path,
                format!(
                    "https://unpkg.com/{}@{}/{name}",
                    package.name, manifest.prisma_version
                ),
            );
        }
    }
    let fingerprint = digest(&serde_json::to_vec(&files).map_err(|error| error.to_string())?);
    let js_dir = cache.join(format!("prisma-{}-{fingerprint}", manifest.prisma_version));
    install_directory(&js_dir, &files, |directory| {
        for (path, url) in &downloads {
            let checksum = files.get(path).ok_or("Missing JavaScript checksum")?;
            write_verified(directory, path, &fetch(url)?, checksum)?;
        }
        Ok(())
    })?;
    Ok(Runner {
        node: node_dir.join("bin/node"),
        script: js_dir.join("prisma/build/index.js"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::sync::{Arc, Barrier};

    type Downloads = BTreeMap<String, Vec<u8>>;

    fn fixture() -> Result<(Manifest, Downloads), Box<dyn std::error::Error>> {
        let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for (name, data) in [
            ("bin/node", b"node fixture".as_slice()),
            ("LICENSE", b"license".as_slice()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, format!("node-fixture/{name}"), data)?;
        }
        let archive = tar.into_inner()?.finish()?;
        let script = b"JavaScript fixture".to_vec();
        let manifest = Manifest {
            node_version: "fixture".into(),
            prisma_version: "fixture".into(),
            nodes: BTreeMap::from([(
                "target".into(),
                NodeArchive {
                    url: "https://fixture/node.tar.gz".into(),
                    sha256: digest(&archive),
                    prefix: "node-fixture".into(),
                    files: BTreeMap::from([
                        ("bin/node".into(), digest(b"node fixture")),
                        ("LICENSE".into(), digest(b"license")),
                    ]),
                },
            )]),
            packages: vec![Package {
                name: "prisma".into(),
                files: BTreeMap::from([("build/index.js".into(), digest(&script))]),
            }],
        };
        Ok((
            manifest,
            BTreeMap::from([
                ("https://fixture/node.tar.gz".into(), archive),
                (
                    "https://unpkg.com/prisma@fixture/build/index.js".into(),
                    script,
                ),
            ]),
        ))
    }

    #[test]
    fn manifest_covers_release_targets_and_entrypoints() -> Result<(), Box<dyn std::error::Error>> {
        let manifest: Manifest = serde_json::from_str(MANIFEST)?;
        assert_eq!(manifest.prisma_version, PRISMA_CLI_VERSION);
        for target in [
            "aarch64-apple-darwin",
            "aarch64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
        ] {
            let node = manifest.nodes.get(target).ok_or("missing target")?;
            assert!(node.files.contains_key("bin/node"));
            assert!(node.files.contains_key("LICENSE"));
            assert!(node.url.starts_with("https://nodejs.org/dist/"));
            assert_eq!(node.sha256.len(), 64);
        }
        for name in ["prisma", "@prisma/engines"] {
            let package = manifest
                .packages
                .iter()
                .find(|p| p.name == name)
                .ok_or("missing package")?;
            assert!(package.files.contains_key("package.json"));
            assert!(package.files.contains_key("LICENSE"));
            for (path, checksum) in &package.files {
                relative_path(path)?;
                assert_eq!(checksum.len(), 64);
                assert!(checksum.bytes().all(|byte| byte.is_ascii_hexdigit()));
            }
        }
        Ok(())
    }

    #[test]
    fn installs_verified_files_and_reuses_cache_offline() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let (manifest, downloads) = fixture()?;
        let runner = install_with(dir.path(), &manifest, "target", |url| {
            downloads.get(url).cloned().ok_or("unknown URL".into())
        })?;
        assert_eq!(fs::read(&runner.node)?, b"node fixture");
        assert_eq!(fs::read(&runner.script)?, b"JavaScript fixture");
        let cached = install_with(dir.path(), &manifest, "target", |_| {
            Err("network must not be used".into())
        })?;
        assert_eq!(cached.node, runner.node);
        assert_eq!(cached.script, runner.script);
        Ok(())
    }

    #[test]
    fn failed_download_never_publishes_partial_javascript() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let (manifest, downloads) = fixture()?;
        let result = install_with(dir.path(), &manifest, "target", |url| {
            if url.ends_with("index.js") {
                return Err("interrupted download".into());
            }
            downloads.get(url).cloned().ok_or("unknown URL".into())
        });
        assert!(result.is_err());
        for entry in fs::read_dir(dir.path())? {
            assert!(entry?.file_name().to_string_lossy().starts_with("node-"));
        }
        install_with(dir.path(), &manifest, "target", |url| {
            assert!(
                url.ends_with("index.js"),
                "verified Node cache should be reused"
            );
            downloads.get(url).cloned().ok_or("unknown URL".into())
        })?;
        Ok(())
    }

    #[test]
    fn rejects_corrupt_download_before_extraction() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let (manifest, _) = fixture()?;
        let result = install_with(dir.path(), &manifest, "target", |_| {
            Ok(b"not an archive".to_vec())
        });
        assert!(matches!(result, Err(error) if error.contains("checksum mismatch")));
        assert_eq!(fs::read_dir(dir.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn rejects_corrupt_cached_node_and_javascript() -> Result<(), Box<dyn std::error::Error>> {
        for corrupt_node in [false, true] {
            let dir = tempfile::tempdir()?;
            let (manifest, downloads) = fixture()?;
            let runner = install_with(dir.path(), &manifest, "target", |url| {
                downloads.get(url).cloned().ok_or("unknown URL".into())
            })?;
            fs::write(
                if corrupt_node {
                    runner.node
                } else {
                    runner.script
                },
                b"tampered",
            )?;
            let result = install_with(dir.path(), &manifest, "target", |_| {
                Err("network must not be used".into())
            });
            assert!(matches!(result, Err(error) if error.contains("checksum mismatch")));
        }
        Ok(())
    }

    #[test]
    fn concurrent_installers_publish_one_complete_directory(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let destination = dir.path().join("installation");
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let destination = destination.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let files = BTreeMap::from([("entry.js".into(), digest(b"complete"))]);
                install_directory(&destination, &files, |temporary| {
                    fs::write(temporary.join("entry.js"), b"complete")
                        .map_err(|error| error.to_string())?;
                    barrier.wait();
                    Ok(())
                })
            }));
        }
        for handle in handles {
            handle.join().map_err(|_| "installer panicked")??;
        }
        assert_eq!(fs::read(destination.join("entry.js"))?, b"complete");
        assert_eq!(fs::read_dir(dir.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn rejects_non_relative_paths() {
        for path in ["", "/absolute", "../escape", "package/../../escape"] {
            assert!(relative_path(path).is_err());
        }
        assert!(relative_path("@prisma/engines/dist/index.js").is_ok());
    }
}
