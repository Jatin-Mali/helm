#!/usr/bin/env sh
set -eu

repo="${HELM_REPO:-https://github.com/white-phantom/helm}"
version="${HELM_VERSION:-latest}"
install_dir="${HELM_INSTALL_DIR:-$HOME/.local/bin}"
state_dir="${HELM_STATE_DIR:-$HOME/.helm}"

mkdir -p "$install_dir" "$state_dir"

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64)   target="x86_64-unknown-linux-gnu" ;;
  aarch64|arm64)  target="aarch64-unknown-linux-gnu" ;;
  *) echo "unsupported architecture: $arch" >&2; exit 2 ;;
esac

if [ "$version" = "latest" ]; then
  url="$repo/releases/latest/download/helm-$target"
else
  url="$repo/releases/download/$version/helm-$target"
fi

tmp="$(mktemp)"
cleanup() { rm -f "$tmp"; }
trap cleanup EXIT

echo "downloading $url"
curl -fsSL "$url" -o "$tmp"
chmod +x "$tmp"
mv "$tmp" "$install_dir/helm"
echo "installed $install_dir/helm"

# Add install_dir to PATH if it isn't already there
add_to_path() {
  shell_rc="$1"
  if [ -f "$shell_rc" ] && grep -qF "$install_dir" "$shell_rc" 2>/dev/null; then
    return 0
  fi
  printf '\nexport PATH="%s:$PATH"\n' "$install_dir" >> "$shell_rc"
  echo "added $install_dir to PATH in $shell_rc"
}

case "${SHELL:-}" in
  */zsh)  add_to_path "$HOME/.zshrc" ;;
  */fish) mkdir -p "$HOME/.config/fish" && \
          printf '\nfish_add_path "%s"\n' "$install_dir" >> "$HOME/.config/fish/config.fish" && \
          echo "added $install_dir to PATH in ~/.config/fish/config.fish" ;;
  *)      add_to_path "$HOME/.bashrc" ;;
esac

if ! command -v helm >/dev/null 2>&1 || [ "$(command -v helm)" != "$install_dir/helm" ]; then
  echo ""
  echo "  NOTE: $install_dir is not in your current PATH."
  echo "  Either restart your shell or run:"
  echo "    export PATH=\"$install_dir:\$PATH\""
fi

echo ""
echo "  Next: helm init"
