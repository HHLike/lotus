#!/usr/bin/env bash
# 构建 Lotus .deb 包
# 用法：./packaging/build-deb.sh
# 产物：dist/lotus_<version>_<arch>.deb

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PKG_NAME="lotus"
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
ARCH="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
MAINTAINER="${LOTUS_MAINTAINER:-Lotus Maintainers <lotus@localhost>}"

DIST_DIR="$ROOT/dist"
STAGE="$DIST_DIR/${PKG_NAME}_${VERSION}_${ARCH}"
DEB_PATH="$DIST_DIR/${PKG_NAME}_${VERSION}_${ARCH}.deb"

echo "==> 清理旧 staging"
rm -rf "$STAGE"
mkdir -p "$STAGE"

echo "==> cargo build --release"
cargo build --release

BIN="$ROOT/target/release/lotus"
if [[ ! -x "$BIN" ]]; then
  echo "错误：找不到 release 二进制 $BIN" >&2
  exit 1
fi

echo "==> 安装文件到 staging"
# 二进制
install -Dm755 "$BIN" "$STAGE/usr/bin/lotus"

# 前端资源
install -d "$STAGE/usr/share/lotus/frontend"
cp -a "$ROOT/frontend/." "$STAGE/usr/share/lotus/frontend/"

# 桌面入口 + 图标
install -Dm644 "$ROOT/packaging/lotus.desktop" \
  "$STAGE/usr/share/applications/lotus.desktop"
install -Dm644 "$ROOT/packaging/lotus.svg" \
  "$STAGE/usr/share/icons/hicolor/scalable/apps/lotus.svg"

# 文档
install -d "$STAGE/usr/share/doc/lotus"
if [[ -f "$ROOT/README.md" ]]; then
  install -Dm644 "$ROOT/README.md" "$STAGE/usr/share/doc/lotus/README.md"
fi
cat > "$STAGE/usr/share/doc/lotus/copyright" <<EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: lotus
Source: local

Files: *
Copyright: $(date +%Y) Lotus Contributors
License: MIT
 Permission is hereby granted, free of charge, to any person obtaining a copy
 of this software and associated documentation files (the "Software"), to deal
 in the Software without restriction, including without limitation the rights
 to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 copies of the Software, and to permit persons to whom the Software is
 furnished to do so, subject to the following conditions:
 .
 The above copyright notice and this permission notice shall be included in all
 copies or substantial portions of the Software.
 .
 THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 SOFTWARE.
EOF

# changelog
cat > "$STAGE/usr/share/doc/lotus/changelog" <<EOF
lotus (${VERSION}) unstable; urgency=low

  * Package build for Lotus terminal.

 -- ${MAINTAINER}  $(date -R)
EOF
gzip -9n -f "$STAGE/usr/share/doc/lotus/changelog"

echo "==> 写 DEBIAN/control"
install -d "$STAGE/DEBIAN"

# 尽量用 dpkg-shlibdeps 推依赖；失败则用保守默认
DEPENDS="libwebkit2gtk-4.1-0, libgtk-3-0, libjavascriptcoregtk-4.1-0, libc6"
if command -v dpkg-shlibdeps >/dev/null 2>&1; then
  # shlibdeps 需要在 staging 里有 DEBIAN，且用临时目录
  set +e
  SHLIBS=$(
    cd "$STAGE" && \
    dpkg-shlibdeps -O -e usr/bin/lotus 2>/dev/null | sed -n 's/^shlibs:Depends=//p'
  )
  set -e
  if [[ -n "${SHLIBS:-}" ]]; then
    DEPENDS="$SHLIBS"
  fi
fi

SIZE_KB="$(du -sk "$STAGE" | awk '{print $1}')"

cat > "$STAGE/DEBIAN/control" <<EOF
Package: ${PKG_NAME}
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${ARCH}
Maintainer: ${MAINTAINER}
Installed-Size: ${SIZE_KB}
Depends: ${DEPENDS}
Recommends: fonts-jetbrains-mono | fonts-dejavu-core
Homepage: https://github.com/local/lotus
Description: OTTY-style native terminal for Linux
 Lotus is a native GUI terminal emulator with a modern dark UI,
 multi-tab sessions, project workspaces, agent launcher, and
 WebKitGTK + xterm.js rendering.
EOF

cat > "$STAGE/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true
fi
exit 0
EOF
chmod 755 "$STAGE/DEBIAN/postinst"

cat > "$STAGE/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = remove ] || [ "$1" = purge ]; then
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true
  fi
fi
exit 0
EOF
chmod 755 "$STAGE/DEBIAN/postrm"

echo "==> dpkg-deb 打包"
# 权限规范化
find "$STAGE" -type d -exec chmod 755 {} +
find "$STAGE/usr" -type f -exec chmod 644 {} +
chmod 755 "$STAGE/usr/bin/lotus"
chmod 755 "$STAGE/DEBIAN/postinst" "$STAGE/DEBIAN/postrm"

fakeroot dpkg-deb --root-owner-group --build "$STAGE" "$DEB_PATH"

echo
echo "✅ 打包完成: $DEB_PATH"
ls -lh "$DEB_PATH"
echo
echo "安装："
echo "  sudo apt install ./$(basename "$DEB_PATH")"
echo "  # 或"
echo "  sudo dpkg -i $DEB_PATH && sudo apt-get install -f -y"
