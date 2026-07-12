#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# Usage: ./release.sh <version>
#   e.g. ./release.sh 0.2.0
# ---------------------------------------------------------------------------

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "Usage: ./release.sh <version>  (e.g. ./release.sh 0.2.0)"
  exit 1
fi

TAG="v$VERSION"

echo "→ Bumping version to $VERSION"

# Cargo.toml (root)
sed -i'' -e "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml

# src-tauri/Cargo.toml
sed -i'' -e "s/^version = \".*\"/version = \"$VERSION\"/" src-tauri/Cargo.toml

# src-tauri/tauri.conf.json
sed -i'' -e "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" src-tauri/tauri.conf.json

echo "→ Committing version bump"
git add Cargo.toml src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump version to $VERSION"

echo "→ Tagging $TAG"
git tag -a "$TAG" -m "Release $TAG"

echo "→ Pushing to origin"
git push origin main --follow-tags

echo ""
echo "✓ Done. GitHub Actions will now build the release."
echo "  Watch progress at: $(git remote get-url origin | sed 's/\.git$//')/actions"
echo "  Draft release at:  $(git remote get-url origin | sed 's/\.git$//')/releases"
