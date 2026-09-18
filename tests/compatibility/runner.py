#!/usr/bin/env python3
"""Check a runner release without production credentials or database migrations."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from urllib.parse import urlparse


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--schema", type=Path, action="append")
    parser.add_argument("--database-url", help="Explicit disposable local Postgres database")
    parser.add_argument("--work-dir", type=Path)
    args = parser.parse_args()
    cli = args.cli.resolve()
    baseline = args.baseline.resolve() if args.baseline else None
    schemas = args.schema or [Path(__file__).with_name("schema.prisma")]
    if args.database_url:
        parsed = urlparse(args.database_url)
        if parsed.hostname not in {"localhost", "127.0.0.1", "fixture-db"}:
            parser.error("database checks only accept local disposable Postgres")
    with tempfile.TemporaryDirectory(prefix="prisma-runner-") as temporary:
        work = args.work_dir.resolve() if args.work_dir else Path(temporary)
        work.mkdir(parents=True, exist_ok=True)
        cache = work / "cache"
        bindir = work / "bin"
        bindir.mkdir(exist_ok=True)
        provider = bindir / "prisma"
        env = dict(os.environ)
        env.update(
            PATH=str(bindir) + os.pathsep + env["PATH"],
            PRISMA_CLI_CACHE_DIR=str(cache),
            PRISMA_HIDE_UPDATE_MESSAGE="true",
            CHECKPOINT_DISABLE="1",
            DATABASE_URL="postgresql://postgres:fixture@localhost:5432/unused",
        )
        for variable in ["PRISMA_GENERATOR_INVOCATION", "PRISMA_QUERY_ENGINE_BINARY",
                         "PRISMA_MIGRATION_ENGINE_BINARY", "PRISMA_INTROSPECTION_ENGINE_BINARY",
                         "PRISMA_FMT_BINARY"]:
            env.pop(variable, None)

        def run(executable, directory, name, arguments, expected_success=True):
            provider.unlink(missing_ok=True)
            provider.symlink_to(executable)
            result = subprocess.run([str(executable), *arguments], cwd=directory, env=env,
                                    capture_output=True, text=True)
            (directory / f"{name}.stdout").write_text(result.stdout)
            (directory / f"{name}.stderr").write_text(result.stderr)
            if expected_success != (result.returncode == 0):
                raise RuntimeError(f"{name} exited {result.returncode}: {result.stderr}")
            return result.stdout

        version = run(cli, work, "version", ["--version"])
        assert "prisma                : 4.8.0" in version
        nodes = list((cache / "runner").glob("node-*/bin/node"))
        assert len(nodes) == 1, "expected exactly one pinned Node runtime"
        runtime = json.loads(subprocess.check_output(
            [str(nodes[0]), "-p", "JSON.stringify(process.versions)"], text=True))
        assert runtime["node"].startswith("24.")
        assert not runtime["openssl"].startswith("1.1.")
        results = {"node": runtime["node"], "openssl": runtime["openssl"], "schemas": []}
        for index, schema in enumerate(schemas):
            directory = work / f"schema-{index}"
            directory.mkdir(exist_ok=True)
            shutil.copy2(schema, directory / "schema.prisma")
            old = None
            if baseline:
                run(baseline, directory, "baseline", ["generate", "--schema", "schema.prisma"])
                old = (directory / "prisma.rs").read_bytes()
            run(cli, directory, "generate", ["generate", "--schema", "schema.prisma"])
            generated = (directory / "prisma.rs").read_bytes()
            if old is not None:
                assert old == generated, f"generated Rust changed for {schema}"
            results["schemas"].append({"schema": str(schema), "bytes": len(generated),
                "sha256": hashlib.sha256(generated).hexdigest(), "baseline_identical": old == generated if old is not None else None})
        run(cli, work, "missing-schema", ["generate", "--schema", "missing.prisma"], False)
        # Existing verified caches remain usable without any network path.
        env.update(HTTPS_PROXY="http://127.0.0.1:1", HTTP_PROXY="http://127.0.0.1:1", ALL_PROXY="http://127.0.0.1:1", NO_PROXY="")
        run(cli, work, "offline", ["--version"])
        for key in ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY", "NO_PROXY"]:
            env.pop(key, None)
        if args.database_url:
            directory = work / "database"
            directory.mkdir(exist_ok=True)
            shutil.copy2(Path(__file__).with_name("schema.prisma"), directory / "schema.prisma")
            env["DATABASE_URL"] = args.database_url
            run(cli, directory, "validate", ["validate", "--schema", "schema.prisma"])
            run(cli, directory, "format", ["format", "--schema", "schema.prisma"])
            sql = run(cli, directory, "diff", ["migrate", "diff", "--from-empty", "--to-schema-datamodel", "schema.prisma", "--script"])
            assert 'CREATE TABLE "Item"' in sql
            migration = directory / "migrations/20260918000000_fixture"
            migration.mkdir(parents=True, exist_ok=True)
            (migration / "migration.sql").write_text(sql)
            (directory / "migrations/migration_lock.toml").write_text('provider = "postgresql"\n')
            run(cli, directory, "deploy", ["migrate", "deploy", "--schema", "schema.prisma"])
            run(cli, directory, "status", ["migrate", "status", "--schema", "schema.prisma"])
            run(cli, directory, "deploy-again", ["migrate", "deploy", "--schema", "schema.prisma"])
            introspected = run(cli, directory, "introspection", ["db", "pull", "--schema", "schema.prisma", "--print"])
            assert "model Item" in introspected and "enum State" in introspected
            diff = run(cli, directory, "db-diff", ["migrate", "diff", "--from-url", args.database_url, "--to-schema-datamodel", "schema.prisma", "--script"])
            assert not any(statement in diff for statement in ["CREATE TABLE", "ALTER TABLE", "DROP TABLE"])
            results["database_checks"] = "passed"
        (work / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
