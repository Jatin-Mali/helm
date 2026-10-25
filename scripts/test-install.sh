#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/fixture" "$tmp/install" "$tmp/home"
cat > "$tmp/bin/uname" <<'MOCK'
#!/bin/sh
case "$1" in -s) echo "${TEST_OS:-Linux}";; -m) echo "${TEST_ARCH:-x86_64}";; esac
MOCK
cat > "$tmp/bin/curl" <<'MOCK'
#!/bin/sh
while [ "$#" -gt 0 ]; do
 case "$1" in -o) out=$2; shift 2;; http*) url=$1; shift;; *) shift;; esac
done
name=${url##*/}
[ "${TEST_FAIL_DOWNLOAD:-}" != "$name" ] || exit 22
cp "$TEST_FIXTURE/$name" "$out"
MOCK
chmod +x "$tmp/bin/"*
export PATH="$tmp/bin:$PATH" HOME="$tmp/home" HELM_INSTALL_DIR="$tmp/install" TEST_FIXTURE="$tmp/fixture"
asset=helm-x86_64-unknown-linux-gnu
printf '#!/bin/sh\nprintf verified\n' > "$tmp/fixture/$asset"
(cd "$tmp/fixture" && sha256sum "$asset" > "$asset.sha256")
old() { printf original > "$tmp/install/helmops"; printf old-alias > "$tmp/install/helm"; }
expect_failure() {
 if sh "$root/install.sh" > "$tmp/output" 2>&1; then cat "$tmp/output"; echo 'installer unexpectedly succeeded' >&2; exit 1; fi
 [ "$(cat "$tmp/install/helmops")" = original ]
 [ "$(cat "$tmp/install/helm")" = old-alias ]
}
old
printf '%064d  %s\n' 0 "$asset" > "$tmp/fixture/$asset.sha256"
expect_failure
(cd "$tmp/fixture" && sha256sum "$asset" > "$asset.sha256")
TEST_FAIL_DOWNLOAD="$asset.sha256" expect_failure
TEST_OS=Darwin expect_failure
TEST_ARCH=aarch64 expect_failure
printf 'invalid checksum\n' > "$tmp/fixture/$asset.sha256"
expect_failure
(cd "$tmp/fixture" && sha256sum "$asset" > "$asset.sha256")
sh "$root/install.sh" > "$tmp/output" 2>&1
[ "$("$tmp/install/helmops")" = verified ]
[ "$("$tmp/install/helm")" = verified ]
[ "$(find "$tmp/install" -maxdepth 1 -name '.helm-install.*' | wc -l)" -eq 0 ]
printf 'installer: mismatch, missing checksum, OS/arch, malformed checksum, success and aliases PASS\n'
