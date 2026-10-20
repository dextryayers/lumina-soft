#!/bin/sh
# Build Lumina's Linux packages from one shared staging tree.
#
# Usage:
#   ./packaging/linux/build.sh --all
#   ./packaging/linux/build.sh --deb --appimage
#   ./packaging/linux/build.sh --stage-only
#
# Environment:
#   VERSION      Override the workspace version.
#   DIST         Output directory (default: <repo>/dist).
#   ARCH         Target architecture label (default: x86_64).
#   SKIP_BUILD   Set to 1 to reuse target/release binaries.

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)
DIST=${DIST:-"$ROOT/dist"}
ARCH=${ARCH:-x86_64}
STAGE="$DIST/linux-stage"
APPDIR="$DIST/Lumina.AppDir"
RPMSRC="$DIST/rpmbuild"

VERSION=${VERSION:-$(awk -F' = ' '/^version = "/ { gsub(/"/, "", $2); print $2; exit }' "$ROOT/Cargo.toml")}
if [ -z "$VERSION" ]; then
    echo "error: cannot read workspace version" >&2
    exit 1
fi

DEB_VERSION=$(printf '%s' "$VERSION" | sed 's/-/~/g')
RPM_VERSION=${VERSION%%-*}
if [ "$RPM_VERSION" = "$VERSION" ]; then
    RPM_RELEASE=1
else
    RPM_RELEASE=$(printf '%s' "${VERSION#*-}" | tr '-' '.')
fi
APK_VERSION=$(printf '%s' "$VERSION" | sed 's/-/_/g')

WANT_ALL=0
WANT_DEB=0
WANT_RPM=0
WANT_APK=0
WANT_APPIMAGE=0
STAGE_ONLY=0
SKIP_BUILD=${SKIP_BUILD:-0}
FAILED=0

usage() {
    cat <<'EOF'
Usage: packaging/linux/build.sh [OPTIONS]

Options:
  --all          Build deb, rpm, apk, and AppImage.
  --deb          Build a .deb.
  --rpm          Build an .rpm.
  --apk          Build an Alpine .apk with abuild.
  --appimage     Build an AppImage with appimagetool.
  --stage-only   Create the shared staging tree and stop.
  --skip-build   Do not run cargo build.
  -h, --help     Show this help.

With no package option, the script behaves like --all.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --all) WANT_ALL=1 ;;
        --deb) WANT_DEB=1 ;;
        --rpm) WANT_RPM=1 ;;
        --apk) WANT_APK=1 ;;
        --appimage) WANT_APPIMAGE=1 ;;
        --stage-only) STAGE_ONLY=1 ;;
        --skip-build) SKIP_BUILD=1 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "error: unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

if [ "$WANT_ALL" -eq 1 ]; then
    WANT_DEB=1
    WANT_RPM=1
    WANT_APK=1
    WANT_APPIMAGE=1
fi

if [ "$WANT_DEB$WANT_RPM$WANT_APK$WANT_APPIMAGE" = "0000" ] && [ "$STAGE_ONLY" -eq 0 ]; then
    WANT_DEB=1
    WANT_RPM=1
    WANT_APK=1
    WANT_APPIMAGE=1
fi

fail() {
    echo "error: $*" >&2
    FAILED=1
}

require() {
    if ! command -v "$1" >/dev/null 2>&1; then
        fail "required tool not found: $1"
        return 1
    fi
    return 0
}

if [ "$SKIP_BUILD" -eq 0 ]; then
    (cd "$ROOT" && cargo build --release --locked -p lumina-app -p lumina-ipc)
fi

for binary in "$ROOT/target/release/lumina" "$ROOT/target/release/lumina-send"; do
    if [ ! -x "$binary" ]; then
        echo "error: missing release binary: $binary" >&2
        exit 1
    fi
done

rm -rf "$STAGE"
mkdir -p "$STAGE/usr/bin"
mkdir -p "$STAGE/usr/share/applications"
mkdir -p "$STAGE/usr/share/metainfo"
mkdir -p "$STAGE/usr/share/icons/hicolor/scalable/apps"
mkdir -p "$STAGE/usr/share/bash-completion/completions"
mkdir -p "$STAGE/usr/share/fish/vendor_completions.d"
mkdir -p "$STAGE/usr/share/zsh/vendor-completions"
mkdir -p "$STAGE/usr/share/doc/lumina"

install -m 0755 "$ROOT/target/release/lumina" "$STAGE/usr/bin/lumina"
install -m 0755 "$ROOT/target/release/lumina-send" "$STAGE/usr/bin/lumina-send"
install -m 0644 "$SCRIPT_DIR/lumina.desktop" "$STAGE/usr/share/applications/lumina.desktop"
install -m 0644 "$SCRIPT_DIR/lumina.appdata.xml" "$STAGE/usr/share/metainfo/lumina.appdata.xml"
install -m 0644 "$SCRIPT_DIR/lumina.svg" "$STAGE/usr/share/icons/hicolor/scalable/apps/lumina.svg"
install -m 0644 "$SCRIPT_DIR/completions/lumina.bash" "$STAGE/usr/share/bash-completion/completions/lumina"
install -m 0644 "$SCRIPT_DIR/completions/lumina-send.bash" "$STAGE/usr/share/bash-completion/completions/lumina-send"
install -m 0644 "$SCRIPT_DIR/completions/lumina.fish" "$STAGE/usr/share/fish/vendor_completions.d/lumina.fish"
install -m 0644 "$SCRIPT_DIR/completions/lumina-send.fish" "$STAGE/usr/share/fish/vendor_completions.d/lumina-send.fish"
install -m 0644 "$SCRIPT_DIR/completions/_lumina" "$STAGE/usr/share/zsh/vendor-completions/_lumina"
install -m 0644 "$SCRIPT_DIR/completions/_lumina-send" "$STAGE/usr/share/zsh/vendor-completions/_lumina-send"
install -m 0644 "$ROOT/LICENSE" "$STAGE/usr/share/doc/lumina/LICENSE"
install -m 0644 "$ROOT/README.md" "$STAGE/usr/share/doc/lumina/README.md"
install -m 0644 "$ROOT/CHANGELOG.md" "$STAGE/usr/share/doc/lumina/CHANGELOG.md"

echo "staged Linux payload in $STAGE"
if [ "$STAGE_ONLY" -eq 1 ]; then
    exit 0
fi

build_deb() {
    require dpkg-deb || return 0
    debroot="$DIST/deb-root"
    rm -rf "$debroot"
    cp -a "$STAGE" "$debroot"
    mkdir -p "$debroot/DEBIAN"
    size=$(du -sk "$debroot" | awk '{print $1}')
    cat > "$debroot/DEBIAN/control" <<EOF
Package: lumina
Version: $DEB_VERSION
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Lumina Contributors <maintainers@lumina.invalid>
Installed-Size: $size
Depends: libc6
Recommends: libvulkan1, libxkbcommon0, libpulse0
Homepage: https://github.com/lumina-sh/lumina
Description: Local-first dynamic island for notifications, media, and system status
 Lumina is a compact desktop overlay with media, notifications, metrics,
 and configurable widgets. It is local-first and has no telemetry.
EOF
    out="$DIST/lumina_${DEB_VERSION}_amd64.deb"
    dpkg-deb --root-owner-group --build "$debroot" "$out"
    echo "built $out"
}

build_rpm() {
    require rpmbuild || return 0
    rm -rf "$RPMSRC"
    mkdir -p "$RPMSRC/SPECS" "$RPMSRC/SOURCES" "$RPMSRC/BUILD" "$RPMSRC/RPMS" "$RPMSRC/SRPMS"
    spec="$RPMSRC/SPECS/lumina.spec"
    cat > "$spec" <<EOF
Name: lumina
Version: $RPM_VERSION
Release: $RPM_RELEASE%{?dist}
Summary: Local-first dynamic island for notifications, media, and system status
License: MIT
URL: https://github.com/lumina-sh/lumina
BuildArch: $ARCH
Requires: glibc
Recommends: vulkan-loader
Recommends: libxkbcommon
Recommends: pulseaudio-libs

%description
Lumina is a compact desktop overlay with media, notifications, metrics,
and configurable widgets. It is local-first and has no telemetry.

%install
rm -rf %{buildroot}
mkdir -p %{buildroot}
cp -a "$STAGE"/. %{buildroot}/

%files
/usr/bin/lumina
/usr/bin/lumina-send
/usr/share/applications/lumina.desktop
/usr/share/metainfo/lumina.appdata.xml
/usr/share/icons/hicolor/scalable/apps/lumina.svg
/usr/share/bash-completion/completions/lumina
/usr/share/bash-completion/completions/lumina-send
/usr/share/fish/vendor_completions.d/lumina.fish
/usr/share/fish/vendor_completions.d/lumina-send.fish
/usr/share/zsh/vendor-completions/_lumina
/usr/share/zsh/vendor-completions/_lumina-send
%doc /usr/share/doc/lumina/README.md /usr/share/doc/lumina/CHANGELOG.md /usr/share/doc/lumina/LICENSE

%changelog
* Tue Oct 20 2026 Lumina Contributors <maintainers@lumina.invalid> - $RPM_VERSION-$RPM_RELEASE
- Alpha engine and headless tooling release.
EOF
    rpmbuild --define "_topdir $RPMSRC" -bb "$spec"
    built=$(find "$RPMSRC/RPMS" -name "lumina-*.rpm" -type f | head -n 1)
    if [ -z "$built" ]; then
        fail "rpmbuild did not produce an RPM"
        return 0
    fi
    out="$DIST/lumina-${VERSION}-${ARCH}.rpm"
    cp "$built" "$out"
    echo "built $out"
}

build_apk() {
    require abuild || return 0
    apkdir="$DIST/apk/lumina"
    rm -rf "$apkdir"
    mkdir -p "$apkdir"
    cat > "$apkdir/APKBUILD" <<EOF
# Maintainer: Lumina Contributors <maintainers@lumina.invalid>
pkgname=lumina
pkgver=$APK_VERSION
pkgrel=0
pkgdesc="Local-first dynamic island for notifications, media, and system status"
url="https://github.com/lumina-sh/lumina"
arch="$ARCH"
license="MIT"
depends=""
makedepends=""
source=""
builddir="\$srcdir"

build() {
    :
}

package() {
    mkdir -p "\$pkgdir"
    cp -a "$STAGE"/. "\$pkgdir"/
}
EOF
    (cd "$apkdir" && abuild -r -P "$DIST/apk/repo")
    built=$(find "$DIST/apk/repo" -name "lumina-*.apk" -type f | head -n 1)
    if [ -z "$built" ]; then
        fail "abuild did not produce an APK"
        return 0
    fi
    out="$DIST/lumina-${VERSION}-${ARCH}.apk"
    cp "$built" "$out"
    echo "built $out"
}

build_appimage() {
    require appimagetool || return 0
    rm -rf "$APPDIR"
    mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/scalable/apps"
    cp "$ROOT/target/release/lumina" "$APPDIR/usr/bin/lumina"
    cp "$ROOT/target/release/lumina-send" "$APPDIR/usr/bin/lumina-send"
    cp "$SCRIPT_DIR/AppRun" "$APPDIR/AppRun"
    cp "$SCRIPT_DIR/lumina.desktop" "$APPDIR/lumina.desktop"
    cp "$SCRIPT_DIR/lumina.desktop" "$APPDIR/usr/share/applications/lumina.desktop"
    cp "$SCRIPT_DIR/lumina.svg" "$APPDIR/lumina.svg"
    cp "$SCRIPT_DIR/lumina.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/lumina.svg"
    chmod 0755 "$APPDIR/AppRun" "$APPDIR/usr/bin/lumina" "$APPDIR/usr/bin/lumina-send"
    out="$DIST/Lumina-${VERSION}-${ARCH}.AppImage"
    ARCH="$ARCH" appimagetool "$APPDIR" "$out"
    echo "built $out"
}

[ "$WANT_DEB" -eq 1 ] && build_deb
[ "$WANT_RPM" -eq 1 ] && build_rpm
[ "$WANT_APK" -eq 1 ] && build_apk
[ "$WANT_APPIMAGE" -eq 1 ] && build_appimage

if command -v sha256sum >/dev/null 2>&1; then
    (cd "$DIST" && find . -maxdepth 1 -type f \( -name "*.deb" -o -name "*.rpm" -o -name "*.apk" -o -name "*.AppImage" \) -print | sort | xargs -r sha256sum > SHA256SUMS)
    echo "wrote $DIST/SHA256SUMS"
fi

if [ "$FAILED" -ne 0 ]; then
    echo "one or more package formats could not be built" >&2
    exit 1
fi
