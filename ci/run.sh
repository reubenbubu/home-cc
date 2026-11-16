#!/usr/bin/env bash
# Local CI, run inside the container (see ci/compose.yml).
#   check  - type-check the screens and run the Rust tests
#   all    - the above, then build the Linux packages into ci/out/ (default)
set -euo pipefail

mode="${1:-all}"
case "$mode" in
  check|all) ;;
  *) echo "usage: ci-run [check|all]" >&2; exit 2 ;;
esac

# Every checkout shares the same Docker volumes, so only one run at a time.
exec 9>/cache/.ci-lock
flock -n 9 || { echo "Another local CI run is in progress; waiting for it..."; flock 9; }

# Work on a copy so nothing is written into your checkout. The copy lives in
# a Docker volume, so node_modules and build caches carry over between runs.
# Files are compared by content and changed ones get a fresh timestamp, so
# cargo always rebuilds what differs, even when switching between checkouts.
rsync -rlpgoD --checksum --delete \
  --exclude .git --exclude node_modules --exclude ci/out \
  --exclude build --exclude .svelte-kit --exclude src-tauri/target \
  /src/ /work/

step() { printf '\n==> %s\n' "$*"; }

step "Installing npm packages"
npm ci --no-audit --no-fund

step "Type-check"
npm run check

step "Rust tests"
cargo test --manifest-path src-tauri/Cargo.toml

if [ "$mode" = all ]; then
  step "Building Linux packages"
  # Clear packages left by earlier versions so only this build is copied out.
  rm -rf "$CARGO_TARGET_DIR/release/bundle"
  npx tauri build --bundles deb,appimage

  bundle="$CARGO_TARGET_DIR/release/bundle"
  rm -f /out/*.deb /out/*.AppImage
  cp "$bundle"/deb/*.deb "$bundle"/appimage/*.AppImage /out/
  # Hand the files to whoever owns the checkout, not root.
  chown "$(stat -c %u:%g /src)" /out /out/*.deb /out/*.AppImage
  step "Packages written to ci/out/"
  ls -lh /out
fi

step "All done"
