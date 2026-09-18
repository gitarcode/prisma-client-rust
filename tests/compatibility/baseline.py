#!/usr/bin/env python3
"""Fetch the checksum-pinned previous release for generator comparisons."""
import hashlib
import io
from pathlib import Path
import sys
import urllib.request
import zipfile

CHECKSUMS = {
    "aarch64-apple-darwin": "f8b1ffd0932f2025da6fac36c410e05eef036778ce858465e9af37359ad4de7b",
    "aarch64-unknown-linux-gnu": "be77d9469fbe0de81c11f95d3c9b92790ad5eefa29ba11da8eda1feb47f63c1e",
    "x86_64-unknown-linux-gnu": "a7f8a122cbe5d1a671fa1dc684a37c266f4508560f011bdf4c847ccf4138bc05",
}
target = sys.argv[1]
url = f"https://github.com/gitarcode/prisma-client-rust/releases/download/v0.6.11.7/prisma-cli-{target}.zip"
data = urllib.request.urlopen(url, timeout=120).read()
assert hashlib.sha256(data).hexdigest() == CHECKSUMS[target], "baseline checksum mismatch"
with zipfile.ZipFile(io.BytesIO(data)) as archive:
    binary = archive.read(f"prisma-cli-{target}")
Path("baseline").mkdir(exist_ok=True)
Path("baseline/prisma").write_bytes(binary)
Path("baseline/prisma").chmod(0o755)
