#!/usr/bin/env bash
# The runtime bundle a release ships (docs/distribution-design.md): the
# runtime crates' source as git holds it, their dependencies vendored, and
# a lockfile that covers every feature a generated program may ask for, so
# a build from the bundle is offline and reproducible.
#
#     scripts/bundle_runtime.sh <version> <out dir>
#         → <out dir>/crow-runtime-<version>.tar.gz and its .sha256
set -euo pipefail
VERSION="${1:?version}"; OUT="${2:?out dir}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/crow-bundle.XXXXXX")"
B="$work/bundle"; mkdir -p "$B" "$OUT"
for c in crow_runtime crow_db crow_web; do
    ( cd "$REPO" && git ls-files -z "$c" | tar --null -T - -cf - ) | tar -xf - -C "$B"
done
# every feature of each crate, so the lockfile and the vendor directory
# hold what any program's manifest can name
features_of() {
    awk '/^\[features\]/{f=1;next} /^\[/{f=0} f && /^[a-z_0-9-]+ *=/{print $1}' "$REPO/$1/Cargo.toml" | grep -v '^default$' | sed 's/.*/"&"/' | paste -sd, -
}
mkdir -p "$B/.probe/src"; echo 'fn main() {}' > "$B/.probe/src/main.rs"
cat > "$B/.probe/Cargo.toml" <<MANIFEST
[package]
name = "crow_probe"
version = "0.0.0"
edition = "2021"

[dependencies]
crow_runtime = { path = "../crow_runtime", features = [$(features_of crow_runtime)] }
crow_db = { path = "../crow_db", features = [$(features_of crow_db)] }
crow_web = { path = "../crow_web", features = [$(features_of crow_web)] }
mimalloc = { version = "0.1", default-features = false }
rayon = "1.11"
serde_json = "1"
serde = { version = "1", features = ["derive"] }
regex = "1"
chrono = "0.4"
tokio = { version = "1", features = ["full"] }
MANIFEST
( cd "$B/.probe" && cargo generate-lockfile --quiet && cargo vendor --versioned-dirs ../vendor >/dev/null )
cp "$B/.probe/Cargo.lock" "$B/Cargo.lock"
rm -rf "$B/.probe"
printf '{"version": "%s", "rust": "%s", "built": "%s"}\n' "$VERSION" "$(grep -o 'RUST_VERSION: &str = "[^"]*"' "$REPO/src/home.rs" | cut -d'"' -f2)" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$B/manifest.json"
tar -czf "$OUT/crow-runtime-$VERSION.tar.gz" -C "$B" .
( cd "$OUT" && { sha256sum "crow-runtime-$VERSION.tar.gz" 2>/dev/null || shasum -a 256 "crow-runtime-$VERSION.tar.gz"; } > "crow-runtime-$VERSION.tar.gz.sha256" )
rm -rf "$work"
echo "wrote $OUT/crow-runtime-$VERSION.tar.gz ($(du -h "$OUT/crow-runtime-$VERSION.tar.gz" | cut -f1))"
