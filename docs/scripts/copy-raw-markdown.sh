#!/usr/bin/env bash
# Copies every docs/src/**/*.md file to its matching path under docs/book/ (mdBook's own output
# directory, which mirrors docs/src/'s structure 1:1 with the same basenames) so raw markdown
# sits alongside every rendered .html page. Run this after `mdbook build`, not instead of it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCS_DIR="$(dirname "$SCRIPT_DIR")"
SRC_DIR="$DOCS_DIR/src"
BOOK_DIR="$DOCS_DIR/book"

if [ ! -d "$BOOK_DIR" ]; then
  echo "error: $BOOK_DIR does not exist — run 'mdbook build docs' first" >&2
  exit 1
fi

count=0
while IFS= read -r -d '' md_file; do
  rel_path="${md_file#"$SRC_DIR"/}"
  dest_path="$BOOK_DIR/$rel_path"
  mkdir -p "$(dirname "$dest_path")"
  cp "$md_file" "$dest_path"
  count=$((count + 1))
done < <(find "$SRC_DIR" -name "*.md" -print0)

echo "copied $count raw markdown file(s) into $BOOK_DIR"
