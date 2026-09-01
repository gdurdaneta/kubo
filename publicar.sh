#!/usr/bin/env bash
# Taguea esta versión: el push del tag dispara el workflow de release, que
# compila Linux, macOS y Windows y publica los binarios acá mismo.
set -euo pipefail

VERSION="${1:-}"
[ -n "$VERSION" ] || { echo "uso: ./publicar.sh v0.2.0 [\"mensaje\"]" >&2; exit 1; }
MENSAJE="${2:-Versión $VERSION}"

git tag -a "$VERSION" -m "$MENSAJE"
git push origin "$VERSION"

REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
echo
echo "compilando: https://github.com/$REPO/actions"
echo "quedará en: https://github.com/$REPO/releases/tag/$VERSION"
