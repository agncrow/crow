#!/usr/bin/env bash
# The runtime bundle a release ships (docs/distribution-design.md): the
# runtime crates' source as git holds it, their dependencies vendored, and
# a lockfile that covers every feature a generated program may ask for, so
# a build from the bundle is offline and reproducible. A vendored package
# that none of the supported targets builds (Windows', WebAssembly's) is
# kept as a stub: cargo reads the manifest of every package the lockfile
# names, but the source only of what it builds.
#
#     scripts/bundle_runtime.sh <version> <out dir>
#         → <out dir>/crow-runtime-<version>.tar.gz and its .sha256
set -euo pipefail
VERSION="${1:?version}"; OUT="${2:?out dir}"
# the targets a release supports (docs/distribution-design.md): another is
# one more word here
TARGETS="aarch64-apple-darwin x86_64-unknown-linux-gnu"
command -v jq >/dev/null || { echo "bundle_runtime.sh needs jq, to read cargo metadata" >&2; exit 1; }
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
# what the targets build: the packages the probe reaches with every
# dependency's platform read for that target — build dependencies and proc
# macros too, which `cargo tree --target` would read for the machine this
# runs on (a build dependency's Linux-only crate is missed on a Mac)
for t in $TARGETS; do
    ( cd "$B/.probe" && cargo metadata --format-version 1 --locked --offline --filter-platform "$t" ) |
        jq -r '.packages[] | select(.source != null) | "\(.name)-\(.version)"'
done | sort -u > "$work/kept"
# (a kept name the vendor directory does not hold means the two name
# packages differently: stop rather than prune the wrong ones)
[ -s "$work/kept" ] || { echo "no vendored package is built for $TARGETS" >&2; exit 1; }
while read -r p; do
    [ -d "$B/vendor/$p" ] || { echo "kept package $p is not in the vendor directory" >&2; exit 1; }
done < "$work/kept"
# every other package: its manifest, an empty lib, and its checksum file
# naming no files, so cargo's verification passes and the lockfile's
# checksum still matches
total=0; stubbed=0
for dir in "$B"/vendor/*/; do
    dir="${dir%/}"; total=$((total + 1))
    grep -qxF "${dir##*/}" "$work/kept" && continue
    sum="$(jq -r '.package // empty' "$dir/.cargo-checksum.json")"
    [ -n "$sum" ] || { echo "no package checksum in $dir/.cargo-checksum.json" >&2; exit 1; }
    find "$dir" -mindepth 1 -maxdepth 1 ! -name Cargo.toml -exec rm -rf {} +
    mkdir "$dir/src" && : > "$dir/src/lib.rs"
    printf '{"files":{},"package":"%s"}\n' "$sum" > "$dir/.cargo-checksum.json"
    stubbed=$((stubbed + 1))
done
# cargo accepts the pruned directory as a generated program's build will
# read it: every manifest, every checksum, the lockfile unchanged, offline
mkdir -p "$B/.probe/.cargo"
printf '[source.crates-io]\nreplace-with = "vendored"\n\n[source.vendored]\ndirectory = "%s"\n\n[net]\noffline = true\n' "$B/vendor" > "$B/.probe/.cargo/config.toml"
for t in $TARGETS; do
    ( cd "$B/.probe" && cargo metadata --format-version 1 --locked --filter-platform "$t" >/dev/null )
done
echo "vendored $total packages; $stubbed of them, built by none of $TARGETS, kept as stubs"
cp "$B/.probe/Cargo.lock" "$B/Cargo.lock"
rm -rf "$B/.probe"
printf '{"version": "%s", "rust": "%s", "built": "%s"}\n' "$VERSION" "$(grep -o 'RUST_VERSION: &str = "[^"]*"' "$REPO/src/home.rs" | cut -d'"' -f2)" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$B/manifest.json"
tar -czf "$OUT/crow-runtime-$VERSION.tar.gz" -C "$B" .
( cd "$OUT" && { sha256sum "crow-runtime-$VERSION.tar.gz" 2>/dev/null || shasum -a 256 "crow-runtime-$VERSION.tar.gz"; } > "crow-runtime-$VERSION.tar.gz.sha256" )
rm -rf "$work"
echo "wrote $OUT/crow-runtime-$VERSION.tar.gz ($(du -h "$OUT/crow-runtime-$VERSION.tar.gz" | cut -f1))"
