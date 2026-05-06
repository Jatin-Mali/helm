#!/usr/bin/env sh
set -eu

repo="${HELM_REPO:-https://github.com/white-phantom/helm}"
version="${HELM_VERSION:-latest}"
install_dir="${HELM_INSTALL_DIR:-$HOME/.local/bin}"
state_dir="${HELM_STATE_DIR:-$HOME/.helm}"

mkdir -p "$install_dir" "$state_dir"

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
  *) echo "unsupported architecture: $arch" >&2; exit 2 ;;
esac

if [ "$version" = "latest" ]; then
  url="$repo/releases/latest/download/helm-$target"
else
  url="$repo/releases/download/$version/helm-$target"
fi

tmp="$(mktemp)"
cleanup() {
  rm -f "$tmp"
}
trap cleanup EXIT

echo "downloading $url"
curl -fsSL "$url" -o "$tmp"
chmod +x "$tmp"
mv "$tmp" "$install_dir/helm"

echo "installed $install_dir/helm"
echo "run: helm init"
