#!/usr/bin/env bash
set -euo pipefail

# Vercel's standard Node.js build image does not ship with mdBook. The docs
# prebuild needs it to render docs/src/ into the static /docs site, so fetch a
# pinned official release only when the binary is unavailable.
MDBOOK_VERSION="0.5.4"
MDBOOK_WORK_DIR="${TMPDIR:-/tmp}/djangors-mdbook-${MDBOOK_VERSION}"

if ! command -v mdbook >/dev/null 2>&1; then
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) ;;
    *)
      echo "Unsupported platform for the pinned mdBook binary: $(uname -s)-$(uname -m)" >&2
      exit 1
      ;;
  esac

  mkdir -p "$MDBOOK_WORK_DIR"
  archive="$MDBOOK_WORK_DIR/mdbook.tar.gz"
  curl --fail --location --silent --show-error \
    "https://github.com/rust-lang/mdBook/releases/download/v${MDBOOK_VERSION}/mdbook-v${MDBOOK_VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
    --output "$archive"
  tar -xzf "$archive" -C "$MDBOOK_WORK_DIR"
  export PATH="$MDBOOK_WORK_DIR:$PATH"
fi

mdbook --version
npm run build
