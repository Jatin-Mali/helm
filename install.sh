#!/usr/bin/env sh
set -eu

repo="${HELM_REPO:-https://github.com/Jatin-Mali/helm}"
version="${HELM_VERSION:-latest}"
install_dir="${HELM_INSTALL_DIR:-$HOME/.local/bin}"
target=x86_64-unknown-linux-gnu

print_source_build_help() {
  cat >&2 <<HELP
A verified Linux x86-64 release asset is required ($repo, $version).
Build from source instead:
  git clone https://github.com/Jatin-Mali/helm.git
  cd helm
  cargo build --release -p helm-cli
  ./target/release/helm init
  ./target/release/helm doctor
HELP
}
fail() { echo "$1" >&2; print_source_build_help; exit 1; }
[ "$(uname -s)" = Linux ] || fail 'Unsupported OS: release binaries support Linux only.'
case "$(uname -m)" in x86_64|amd64) ;; *) fail 'Unsupported architecture: only Linux x86-64 releases are verified.' ;; esac
command -v sha256sum >/dev/null 2>&1 || fail 'sha256sum is required to verify the release.'
command -v curl >/dev/null 2>&1 || fail 'curl is required to download the release.'

asset="helm-$target"
if [ "$version" = latest ]; then
  url="$repo/releases/latest/download/$asset"
else
  url="$repo/releases/download/$version/$asset"
fi
mkdir -p "$install_dir"
[ ! -d "$install_dir/helmops" ] && [ ! -d "$install_dir/helm" ] || fail 'Install destination is a directory.'
stage="$(mktemp -d "$install_dir/.helm-install.XXXXXX")"
cleanup() { rm -rf -- "$stage"; }
trap cleanup EXIT HUP INT TERM

echo "downloading $url"
curl --proto '=https' --proto-redir '=https' -fsSL "$url" -o "$stage/$asset" || fail 'Release download failed; existing installation preserved.'
curl --proto '=https' --proto-redir '=https' -fsSL "$url.sha256" -o "$stage/checksum" || fail 'Checksum download failed; existing installation preserved.'
expected=$(awk -v name="$asset" 'NF != 2 || $2 != name || length($1) != 64 || $1 ~ /[^[:xdigit:]]/ { exit 1 } { digest=$1 } END { if (NR != 1) exit 1; print tolower(digest) }' "$stage/checksum") || fail 'Malformed release checksum; existing installation preserved.'
actual=$(sha256sum "$stage/$asset")
actual=${actual%% *}
[ "$expected" = "$actual" ] || fail 'Release checksum mismatch; existing installation preserved.'
chmod 755 "$stage/$asset"
ln -s helmops "$stage/helm"
# Stage on the destination filesystem so replacing the verified binary is atomic.
mv -f "$stage/$asset" "$install_dir/helmops"
mv -f "$stage/helm" "$install_dir/helm"
echo "installed $install_dir/helmops (SHA-256 verified)"
echo "compatibility alias: $install_dir/helm"
echo "Add $install_dir to PATH if needed, then run: helm init"
