#!/usr/bin/env bash
# Installs Emperor Mod Manager on Arch-based distros as a pacman package.
#
#   curl -fsSL https://raw.githubusercontent.com/LewisTansley/emperor-mod-manager/main/scripts/install-arch.sh | bash
#
# Env: EMM_VERSION=0.4.0 pins a version instead of using the latest release.
set -euo pipefail

REPO="LewisTansley/emperor-mod-manager"
PKGNAME="emperor-mod-manager-bin"

die() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

info() {
  printf '==> %s\n' "$1"
}

[ "$(id -u)" -ne 0 ] || die "run this as your normal user, not root; makepkg refuses to run as root and will ask for sudo itself"
command -v pacman >/dev/null 2>&1 || die "pacman not found; this installer is for Arch-based distros. Use the .AppImage or .deb from https://github.com/$REPO/releases"
[ "$(uname -m)" = "x86_64" ] || die "only x86_64 builds are published (this machine is $(uname -m))"
command -v curl >/dev/null 2>&1 || die "curl is required: sudo pacman -S --needed curl"

missing=()
for tool in makepkg fakeroot bsdtar; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if [ ${#missing[@]} -gt 0 ]; then
  die "missing build tools (${missing[*]}); install them with: sudo pacman -S --needed base-devel"
fi

# `curl | bash` leaves stdin on the pipe, so pacman's prompts would read EOF.
if [ ! -t 0 ] && [ -e /dev/tty ] && { : >/dev/tty; } 2>/dev/null; then
  exec </dev/tty
fi

version="${EMM_VERSION:-}"
if [ -z "$version" ]; then
  info "Resolving the latest release"
  version="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' |
    head -n1)"
  [ -n "$version" ] || die "could not resolve the latest release tag; set EMM_VERSION=<x.y.z> and retry"
fi
tag="v${version#v}"
version="${version#v}"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
cd "$workdir"

info "Fetching PKGBUILD for $tag"
if ! curl -fsSL -o PKGBUILD "https://github.com/$REPO/releases/download/$tag/PKGBUILD"; then
  # Releases before the PKGBUILD asset existed: use the in-repo copy at that tag
  # and take the checksum from the release's SHA256SUMS.
  info "No PKGBUILD asset on $tag; falling back to the in-repo PKGBUILD"
  curl -fsSL -o PKGBUILD \
    "https://raw.githubusercontent.com/$REPO/$tag/packaging/arch/PKGBUILD" ||
    die "no PKGBUILD available for $tag"

  sha256=""
  if curl -fsSL -o SHA256SUMS "https://github.com/$REPO/releases/download/$tag/SHA256SUMS"; then
    sha256="$(awk -v f="emperor-mod-manager-$version-x86_64.tar.gz" '$2 == f { print $1 }' SHA256SUMS)"
  fi
  [ -n "$sha256" ] || die "no checksum published for $tag; install manually from https://github.com/$REPO/releases/tag/$tag"

  sed -i "s/^pkgver=.*/pkgver=$version/;s/^sha256sums=('SKIP')/sha256sums=('$sha256')/" PKGBUILD
fi

grep -q "^pkgname=$PKGNAME\$" PKGBUILD || die "downloaded PKGBUILD does not look like $PKGNAME; refusing to build it"

makepkg_args=(-si --clean)
[ -t 0 ] || makepkg_args+=(--noconfirm)

info "Building and installing $PKGNAME $version (sudo is needed for the pacman step)"
makepkg "${makepkg_args[@]}"

cat <<EOF

==> Installed. Launch it from your app menu or run: emperor-mod-manager
==> "Download with manager" (nxm://) links open in the app automatically.
==> Update: re-run this installer. Uninstall: sudo pacman -R $PKGNAME
EOF
