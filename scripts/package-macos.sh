#!/bin/sh
# Builds a universal (Apple silicon + Intel) Rigometry.app and a verified ZIP.
# Usage: scripts/package-macos.sh [package-name]   Run on macOS with Rustup and Xcode tools.
set -eu

[ "$(uname -s)" = Darwin ] || { echo 'Packaging the app bundle requires macOS.' >&2; exit 1; }
cd "$(dirname "$0")/.."
root=$(pwd -P)

version=$(sed -n 's/^version = "\([0-9A-Za-z.+-]*\)"$/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || { echo 'Cannot read the application version from Cargo.toml.' >&2; exit 1; }
name=${1:-Rigometry-$version-macos-universal}
case $name in
  ''|-*|*[!A-Za-z0-9._-]*) echo "Invalid package name: $name" >&2; exit 1 ;;
esac
dist="$root/dist"
package="$dist/$name"
archive="$dist/$name.zip"
# Never replace an earlier package or published file.
[ ! -e "$package" ] && [ ! -e "$archive" ] || { echo "Output already exists: $package" >&2; exit 1; }

targets='aarch64-apple-darwin x86_64-apple-darwin'
# 11.0 is the first macOS release on Apple silicon; Intel builds share it.
export MACOSX_DEPLOYMENT_TARGET=11.0
for target in $targets; do
  cargo build --release --locked --target "$target"
done

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
binary="$work/rigometry"
lipo -create -output "$binary" \
  target/aarch64-apple-darwin/release/rigometry \
  target/x86_64-apple-darwin/release/rigometry
for arch in arm64 x86_64; do
  lipo "$binary" -verify_arch "$arch"
done
reported=$("$binary" --version)
[ "$reported" = "Rigometry $version" ] || { echo "Executable reports '$reported', expected 'Rigometry $version'." >&2; exit 1; }

# Icon set from the 512 px master icon.
iconset="$work/rigometry.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" assets/rigometry.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  if [ "$double" -le 512 ]; then
    sips -z "$double" "$double" assets/rigometry.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
  fi
done
iconutil --convert icns --output "$work/rigometry.icns" "$iconset"

mkdir -p "$package"
app="$package/Rigometry.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/rigometry"
cp "$work/rigometry.icns" "$app/Contents/Resources/rigometry.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>de</string><string>fr</string><string>es</string></array>
  <key>CFBundleDisplayName</key><string>Rigometry</string>
  <key>CFBundleExecutable</key><string>rigometry</string>
  <key>CFBundleIconFile</key><string>rigometry</string>
  <key>CFBundleIdentifier</key><string>de.marlonkaulich.rigometry</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Rigometry</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key><string>$MACOSX_DEPLOYMENT_TARGET</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>Copyright (c) 2026 Marlon Kaulich. MIT license.</string>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null
# Ad-hoc signature: required to run on Apple silicon; it is not a Developer ID.
codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict --verbose=1 "$app"

for relative in README.md LICENSE SECURITY.md CHANGELOG.md docs/usage.md \
  docs/dependency-notices.md docs/third-party-licenses.txt \
  docs/toolchain-notices/rust-1.99.0/README.md \
  docs/toolchain-notices/rust-1.99.0/COPYRIGHT-library.html; do
  mkdir -p "$package/$(dirname "$relative")"
  cp "$relative" "$package/$relative"
done

(
  cd "$package"
  find . -type f ! -name SHA256SUMS.txt | LC_ALL=C sort | sed 's|^\./||' | while IFS= read -r file; do
    shasum -a 256 "$file"
  done > SHA256SUMS.txt
  shasum -a 256 --check --strict --quiet SHA256SUMS.txt
)
ditto -c -k --sequesterRsrc --keepParent "$package" "$archive"
(cd "$dist" && shasum -a 256 "$name.zip" > "$name.zip.sha256" && shasum -a 256 --check --strict "$name.zip.sha256")

# Verify that the archive unpacks to a runnable, correctly signed bundle.
ditto -x -k "$archive" "$work/unpacked"
codesign --verify --strict "$work/unpacked/$name/Rigometry.app"
[ "$("$work/unpacked/$name/Rigometry.app/Contents/MacOS/rigometry" --version)" = "Rigometry $version" ]
(cd "$work/unpacked/$name" && shasum -a 256 --check --strict --quiet SHA256SUMS.txt)
echo "$archive"
