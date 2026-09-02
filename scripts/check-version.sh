#!/bin/sh
# VERSION must be the single source of truth for the release version.
# Fails if a SemVer literal is hardcoded anywhere it could drift.
set -eu

VERSION=$(cat VERSION)
STATUS=0

if ! printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "VERSION is not bare SemVer: '$VERSION'" >&2
    exit 1
fi

# The workspace manifest is the one place the version may appear literally.
if ! grep -q "^version = \"$VERSION\"" Cargo.toml; then
    echo "Cargo.toml [workspace.package] version does not match VERSION ($VERSION)" >&2
    STATUS=1
fi

# Member crates must inherit, never restate.
for manifest in crates/*/Cargo.toml; do
    if grep -Eq '^version = "[0-9]+\.[0-9]+\.[0-9]+"' "$manifest"; then
        echo "$manifest hardcodes a version; use 'version.workspace = true'" >&2
        STATUS=1
    fi
done

# No SemVer literals in source.
if grep -rEn '"[0-9]+\.[0-9]+\.[0-9]+"' crates/*/src >/dev/null 2>&1; then
    echo "SemVer literal found in source; derive it from CARGO_PKG_VERSION:" >&2
    grep -rEn '"[0-9]+\.[0-9]+\.[0-9]+"' crates/*/src >&2
    STATUS=1
fi

[ "$STATUS" -eq 0 ] && echo "version check ok: $VERSION ($(cat RELEASE_NAME))"
exit "$STATUS"
