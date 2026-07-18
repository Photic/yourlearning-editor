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
sed -i '' "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml

# src-tauri/Cargo.toml
sed -i '' "s/^version = \".*\"/version = \"$VERSION\"/" src-tauri/Cargo.toml

# src-tauri/tauri.conf.json
sed -i '' "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" src-tauri/tauri.conf.json

echo "→ Committing version bump"
git add Cargo.toml src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump version to $VERSION"

echo "→ Tagging $TAG"
git tag -a "$TAG" -m "Release $TAG"

echo "→ Pushing to origin"
git push origin main --follow-tags

REPO_URL="$(git remote get-url origin | sed 's/\.git$//')"

echo ""
echo "✓ Tag pushed. Waiting for GitHub Actions to finish building the draft release…"
echo "  Watch progress at: $REPO_URL/actions"
echo ""

# Poll until the draft release for this tag exists, then publish it.
echo "→ Waiting for draft release $TAG to appear on GitHub…"
for i in $(seq 1 120); do
  STATUS="$(gh release view "$TAG" --json isDraft --jq '.isDraft' 2>/dev/null || true)"
  if [[ "$STATUS" == "true" ]]; then
    break
  elif [[ "$STATUS" == "false" ]]; then
    echo "  Release $TAG is already published."
    exit 0
  fi
  echo "  ($i/120) Not ready yet, retrying in 30 s…"
  sleep 30
done

if [[ "$(gh release view "$TAG" --json isDraft --jq '.isDraft' 2>/dev/null || true)" != "true" ]]; then
  echo "✗ Timed out waiting for the draft release. Publish it manually at:"
  echo "  $REPO_URL/releases"
  exit 1
fi

RELEASE_NOTES='See the assets below to download and install this version.

Since I do not have a paid account for Apple applications. A small command need to be run after the application has been moved to the application folder.

`xattr -cr /Applications/owls.app`

Similar work arounds might be required for other operation systems.'

echo "→ Publishing release $TAG"
gh release edit "$TAG" --draft=false --notes "$RELEASE_NOTES"

echo ""
echo "✓ Release $TAG is now public."
echo "  $REPO_URL/releases/tag/$TAG"
