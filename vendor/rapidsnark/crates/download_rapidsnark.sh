#!/bin/sh

# Exit on error
set -e

# OUT_DIR is specified by the rust build environment
if [ -z "$OUT_DIR" ]; then
    echo "OUT_DIR not specified"
    exit 1
fi
# Use Unix-style paths on Windows to avoid path issues.
if command -v cygpath > /dev/null 2>&1; then
    OUT_DIR=$(cygpath -u "$OUT_DIR")
fi
# TARGET is specified by the rust build environment
if [ -z "$TARGET" ]; then
    echo "TARGET not specified"
    exit 1
fi

RAPIDSNARK_VERSION=v0.0.8
# The Logos rapidsnark fork's release of the iden3 version above, used for static desktop builds.
# When upstream changes RAPIDSNARK_VERSION, release the fork for that version first, then update this.
LOGOS_RAPIDSNARK_SUFFIX=logos.1-rc.4
LOGOS_RAPIDSNARK_VERSION=$RAPIDSNARK_VERSION-$LOGOS_RAPIDSNARK_SUFFIX
if ! echo "$LOGOS_RAPIDSNARK_VERSION" | grep -Eqx 'v[0-9]+\.[0-9]+\.[0-9]+-logos\.[0-9]+(-rc\.[0-9]+)?'; then
    echo "Malformed LOGOS_RAPIDSNARK_VERSION: $LOGOS_RAPIDSNARK_VERSION"
    exit 1
fi

IDEN3_BASE_URL=https://github.com/iden3/rapidsnark/releases/download/$RAPIDSNARK_VERSION
LOGOS_BASE_URL=https://github.com/logos-blockchain/logos-blockchain-rapidsnark/releases/download/$LOGOS_RAPIDSNARK_VERSION

BUILD_DIR=$OUT_DIR/rapidsnark
LIB_DIR=$BUILD_DIR/$TARGET

REQUIRED_LIBS="librapidsnark.a libfr.a libfq.a libgmp.a"

ARCH=$(echo "$TARGET" | cut -d'-' -f1)

# Apple assets are universal archives, which rustc rejects when bundling a static
# library ("Unsupported archive identifier"), so thin them to LIPO_ARCH.
#
# What build.rs links, and where it comes from:
# - Mobile: the static libraries, from iden3.
# - Desktop with static-rapidsnark: the static libraries, from the Logos fork.
# - Desktop without static-rapidsnark: the shared library, from iden3, so SHARED_LIB is set.
# - Windows: the static libraries, from the Logos fork. iden3 doesn't publish Windows builds.
LIPO_ARCH=
SHARED_LIB=
# Who publishes the static libraries: iden3, or the Logos fork.
STATIC_SOURCE=iden3
case "$TARGET" in
    aarch64-apple-ios)
        SLUG=iOS
        LIPO_ARCH=arm64
        ;;
    aarch64-apple-ios-sim)
        SLUG=iOS-Simulator
        LIPO_ARCH=arm64
        ;;
    # Rejected by ensure_arch: the simulator libfr.a and libfq.a are arm64 only.
    x86_64-apple-ios)
        SLUG=iOS-Simulator
        LIPO_ARCH=x86_64
        ;;
    aarch64-apple-darwin)
        SLUG=macOS-arm64
        LIPO_ARCH=arm64
        SHARED_LIB=librapidsnark.dylib
        STATIC_SOURCE=logos
        ;;
    x86_64-apple-darwin)
        SLUG=macOS-x86_64
        LIPO_ARCH=x86_64
        SHARED_LIB=librapidsnark.dylib
        STATIC_SOURCE=logos
        ;;
    x86_64-pc-windows-gnu)
        SLUG=windows-x86_64
        STATIC_SOURCE=logos
        ;;
    *-windows-*)
        echo "No rapidsnark prebuilt covers target $TARGET. On Windows, only x86_64-pc-windows-gnu is supported."
        echo "Set RAPIDSNARK_LIB_DIR to a directory containing $REQUIRED_LIBS to build against your own."
        exit 1
        ;;
    aarch64-linux-android) SLUG=android-arm64 ;;
    x86_64-linux-android) SLUG=android-x86_64 ;;
    *)
        case "$ARCH" in
            aarch64 | arm64)
                SLUG=linux-arm64
                SHARED_LIB=librapidsnark.so
                STATIC_SOURCE=logos
                ;;
            x86_64 | amd64)
                SLUG=linux-x86_64
                SHARED_LIB=librapidsnark.so
                STATIC_SOURCE=logos
                ;;
            *)
                echo "No rapidsnark prebuilt covers target $TARGET (architecture $ARCH)."
                echo "Set RAPIDSNARK_LIB_DIR to a directory containing $REQUIRED_LIBS to build against your own."
                exit 1
                ;;
        esac
        echo "No prebuilt matches $TARGET exactly, falling back to the generic $SLUG build."
        ;;
esac

if [ "$RAPIDSNARK_STATIC" = 1 ]; then
    SOURCE=$STATIC_SOURCE
    SHARED_LIB=
else
    SOURCE=iden3
fi

if [ "$SOURCE" = logos ]; then
    ASSET=rapidsnark-$SLUG-$LOGOS_RAPIDSNARK_VERSION.tar.gz
    ASSET_BASE_URL=$LOGOS_BASE_URL
else
    ASSET=rapidsnark-$SLUG-$RAPIDSNARK_VERSION.zip
    ASSET_BASE_URL=$IDEN3_BASE_URL
fi
BASE_URL=${RAPIDSNARK_DOWNLOAD_BASE_URL:-$ASSET_BASE_URL}

# Pinned per version, so update them on every bump. Regenerate with:
#   gh api repos/iden3/rapidsnark/releases/tags/$RAPIDSNARK_VERSION \
#     --jq '.assets[] | "\(.name) \(.digest)"'
#   gh api repos/logos-blockchain/logos-blockchain-rapidsnark/releases/tags/$LOGOS_RAPIDSNARK_VERSION \
#     --jq '.assets[] | "\(.name) \(.digest)"'
case "$SLUG" in
    iOS) SHA256=2015bb8825f37356f70aa1a3a7fd66e08bb1205a174d3793bfdf388a9e7e6cfd ;;
    iOS-Simulator) SHA256=6455825c56b03a9001a99aae303541f7565e4587b023223a5e4b148edf203cfa ;;
    macOS-arm64) SHA256=dbd2c1498663223232f9c3ad02259d2839e62e784e9b1f6a0e9bd5070443990d ;;
    macOS-x86_64) SHA256=df116044e6edfd409aa198a9bc828f0f038ddfeaa3243d350460da3a08376631 ;;
    android-arm64) SHA256=2e306704fe1b900261006ec41a7bef92bb3f94839c9989eff5058b133f5dc642 ;;
    android-x86_64) SHA256=966d0e572963af1ff35faa56a5a16754a67000308d0cd89ed6c4d60a3080a2e0 ;;
    linux-arm64) SHA256=704dfbaa6847d4ddf5f63bf7bc8d3e59f007c33e2d8ab16b318090d671253dbd ;;
    linux-x86_64) SHA256=2ec59e3aa5ff498e862d60b3b7abdcd094ea484271750ec1ea14fb7c1305e423 ;;
esac

# The Logos fork's archives.
if [ "$SOURCE" = logos ]; then
    case "$SLUG" in
        linux-arm64) SHA256=f093b0ca6121b8c8af56b39e78045be6526b8990bdb4e73bc3ca0b758909940d ;;
        linux-x86_64) SHA256=6193ff030f0810478c0b165a07a1e1d5857fc3ec7d6f98f0f97d6280047c0e91 ;;
        macOS-arm64) SHA256=0c6e8cef95e55c1c952a82bb412650c3cad6a696b5e3a4f5041a486887d41108 ;;
        macOS-x86_64) SHA256=4ad188b832773f2a24656c6b284789747e723548ae8525db1f7f20e976077374 ;;
        windows-x86_64) SHA256=9cc8a0b3c7e5394f1c4204f69cccbc58806346e4d7c5cda176963fbc0718c776 ;;
    esac
fi

verify_checksum() {
    archive="$1"
    expected="$2"

    if command -v shasum > /dev/null 2>&1; then
        actual=$(shasum -a 256 "$archive" | cut -d' ' -f1)
    else
        actual=$(sha256sum "$archive" | cut -d' ' -f1)
    fi

    if [ "$actual" != "$expected" ]; then
        echo "Checksum mismatch for $archive"
        echo "  expected $expected"
        echo "  actual   $actual"
        return 1
    fi
}

# Upstream coverage is uneven per library, so reject a mismatch here rather than
# leave it to surface when an app links the rlib.
ensure_arch() {
    lib_file="$1"
    arch_name="$2"

    info=$(lipo -info "$lib_file" 2> /dev/null) || return 0

    case "$info" in
        'Architectures in the fat file'*)
            if ! echo "$info" | tr ' ' '\n' | grep -qx "$arch_name"; then
                echo "$(basename "$lib_file") in $ASSET has no $arch_name slice."
                report_missing_arch "$arch_name"
                return 1
            fi
            lipo -thin "$arch_name" -output "$lib_file.thin" "$lib_file" || return 1
            mv "$lib_file.thin" "$lib_file"
            ;;
        *)
            actual=${info##*: }
            if [ "$actual" != "$arch_name" ]; then
                echo "$(basename "$lib_file") in $ASSET is $actual, but $TARGET needs $arch_name."
                report_missing_arch "$arch_name"
                return 1
            fi
            ;;
    esac
}

report_missing_arch() {
    echo "Upstream does not publish an $1 build of every library in $ASSET."
    echo "Set RAPIDSNARK_LIB_DIR to a directory holding $1 builds of $REQUIRED_LIBS to build this target."
}

# Every library, so an interrupted build cannot look finished, and from the same
# asset, since static and shared builds use different archives.
unpacked=yes
if [ "$(cat "$LIB_DIR/.asset" 2> /dev/null)" != "$ASSET" ]; then
    unpacked=no
fi
for lib in $REQUIRED_LIBS $SHARED_LIB; do
    if [ ! -f "$LIB_DIR/$lib" ]; then
        unpacked=no
        break
    fi
done
if [ "$unpacked" = yes ]; then
    exit 0
fi
rm -rf "$LIB_DIR"

mkdir -p "$LIB_DIR"
ARCHIVE=$BUILD_DIR/$ASSET

echo "Downloading $ASSET from $BASE_URL"
if ! curl -fSL -o "$ARCHIVE" "$BASE_URL/$ASSET"; then
    echo "Failed to download $BASE_URL/$ASSET"
    echo "Set RAPIDSNARK_DOWNLOAD_BASE_URL to use a mirror, or RAPIDSNARK_LIB_DIR to build against local libraries."
    exit 1
fi

verify_checksum "$ARCHIVE" "$SHA256"

EXTRACT_DIR=$BUILD_DIR/extract-$SLUG
rm -rf "$EXTRACT_DIR"
mkdir -p "$EXTRACT_DIR"

echo "Extracting $ASSET"
case "$ASSET" in
    *.tar.gz) tar -xzf "$ARCHIVE" -C "$EXTRACT_DIR" ;;
    *) unzip -q "$ARCHIVE" -d "$EXTRACT_DIR" ;;
esac

# The iOS assets keep these at the top level, other platforms nest them in lib/.
for lib in $REQUIRED_LIBS $SHARED_LIB; do
    src=$(find "$EXTRACT_DIR" -type f -name "$lib" | head -1)
    if [ -z "$src" ]; then
        echo "$ASSET does not contain $lib"
        rm -rf "$LIB_DIR" "$EXTRACT_DIR"
        exit 1
    fi
    cp "$src" "$LIB_DIR/$lib"

    if [ -n "$LIPO_ARCH" ] && ! ensure_arch "$LIB_DIR/$lib" "$LIPO_ARCH"; then
        rm -rf "$LIB_DIR" "$EXTRACT_DIR"
        exit 1
    fi
done

rm -rf "$EXTRACT_DIR" "$ARCHIVE"
echo "$ASSET" > "$LIB_DIR/.asset"

echo "rapidsnark libraries for $TARGET ready in $LIB_DIR"
