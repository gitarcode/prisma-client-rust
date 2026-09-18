use crate::binaries::{self, platform, ENGINES};
use std::env;
use std::process::Command;

pub fn main(args: &[String]) -> Result<i32, String> {
    let dir = binaries::global_cache_dir()?;

    let runner = binaries::fetch_native(&dir)?;
    let mut cmd = Command::new(runner.node);
    cmd.arg(runner.script);
    let binary_name = platform::binary_platform_name()?;

    cmd.args(args);

    cmd.envs(env::vars());
    cmd.env("PRISMA_HIDE_UPDATE_MESSAGE", "true");
    cmd.env("PRISMA_CLI_QUERY_ENGINE_TYPE", "binary");

    for e in ENGINES {
        match env::var(e.env) {
            Ok(path) => {
                cmd.env(e.env, path);
            }
            Err(_) => {
                let path = dir
                    .join(binaries::ENGINE_VERSION)
                    .join(format!("prisma-{}-{}", e.name, binary_name));
                cmd.env(e.env, path);
            }
        }
    }

    cmd.stdout(std::process::Stdio::inherit());
    cmd.stdin(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    let status = cmd
        .status()
        .map_err(|error| format!("Cannot start Prisma CLI: {error}"))?;
    Ok(status.code().unwrap_or(1))
}
