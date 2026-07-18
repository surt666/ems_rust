#!/usr/bin/env bash
# Build the query-raw-duck bootstrap INSIDE amazonlinux:2023, so it links AL2023's
# GNU libc/libstdc++ dynamically — the same C++ runtime the DuckDB extensions (GCC
# .so's) use. A zig/cargo-lambda build instead statically embeds LLVM libc++, whose
# ABI collides with the extensions on dlopen and hard-crashes LOAD. Output:
#   target/lambda/query-raw-duck/bootstrap   (x86_64, glibc 2.34, GNU libstdc++)
# The AL2023 libstdc++.so.6 / libgcc_s.so.1 already sit in that dir's lib/.
set -euo pipefail

REPO=/home/sla/projects/ems_rust
OUT="$REPO/target/lambda/query-raw-duck"
mkdir -p "$OUT"

docker run --rm \
  -v "$REPO":/src:ro \
  -v "$OUT":/out \
  -v duckbuild-target:/build \
  -v duckbuild-cargo:/root/.cargo \
  -w /src \
  amazonlinux:2023 \
  bash -euo pipefail -c '
    dnf install -y gcc gcc-c++ make cmake openssl-devel pkgconfig perl tar gzip which findutils >/dev/null
    if ! command -v cargo >/dev/null; then
      curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal >/dev/null
    fi
    source /root/.cargo/env
    export CARGO_TARGET_DIR=/build
    cargo build --release -p query-raw-duck
    cp /build/release/query-raw-duck /out/bootstrap
    echo "=== built bootstrap ==="
    file /out/bootstrap
    echo "=== NEEDED libs ==="
    readelf -d /out/bootstrap | grep NEEDED
  '

# NB: extensions are NOT bundled — the lambda INSTALLs them at runtime into /tmp. Bundling
# the 116M of extensions in the package was measured to *increase* cold start (~6s→~10s;
# read from read-only /var/task is slower than Lambda's network download + /tmp load), so
# it was reverted. Keep the package lean.
