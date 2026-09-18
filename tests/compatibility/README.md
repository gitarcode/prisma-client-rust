# Prisma runner compatibility

The CLI uses checksum-pinned Node 24 with Prisma 4.8 JavaScript. It retains the
Rust generator and the PostgreSQL engines from `v4.8.0-gitar.1`.

`cli/src/binaries/runner.json` pins the Node archives and every file from the
`prisma` and `@prisma/engines` packages. JavaScript files come from versioned UNPKG
URLs. No npm lifecycle scripts run. The Node license and package licenses remain
in the installed directories.

Downloads install into temporary directories and become visible after checksum
verification and an atomic rename. Cache keys include content hashes. Each launch
verifies cached files, and a corrupt cache fails with the directory to remove.
Parallel installers reuse the completed installation from the first successful rename.

The cache defaults to the platform cache directory under `prisma/binaries/cli/4.8.0`.
`PRISMA_CLI_CACHE_DIR` accepts an absolute override, including an empty directory
for release tests. The old photongo executable is never selected.

## Run the checks

Build `prisma-cli` with the PostgreSQL features, then run:

```sh
cargo test --locked -p prisma-client-rust-cli --no-default-features --features postgresql,mocking --lib binaries
cargo build --locked -p prisma-cli
python3 tests/compatibility/runner.py target/debug/prisma --baseline /path/to/v0.6.11.7/prisma
```

Pass `--schema` repeatedly to compare additional Gitar schemas. Each schema must
use the `prisma` provider and write `prisma.rs`. `--work-dir` retains command logs
and the result manifest. The baseline and replacement run at the same schema path
so generated Rust can be compared byte-for-byte.

`--database-url` enables migration deployment and introspection on a disposable
PostgreSQL database. Only localhost or the Docker hostname `fixture-db` is accepted.
The runner does not read an ambient database URL for these checks.

Linux ARM64 still needs the system `openssl` command for Prisma 4.8 platform
detection. Retain maintained certificate packages and OpenSSL. This runtime change
removes the obsolete embedded Node/OpenSSL executable, not that platform probe.
