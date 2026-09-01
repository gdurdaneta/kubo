#!/usr/bin/env bash
# Taguea esta versión y dispara la build pública.
#
# El repo de descargas (público) clona este (privado) con una deploy key de
# solo lectura, compila los tres sistemas y publica ahí la release. Se dispara
# a mano a propósito: automatizarlo requeriría guardar acá un token con permiso
# de escritura sobre el repo público, y no vale la pena por un comando.
set -euo pipefail

VERSION="${1:-}"
[ -n "$VERSION" ] || { echo "uso: ./publicar.sh v0.2.0 [\"mensaje\"]" >&2; exit 1; }
MENSAJE="${2:-Versión $VERSION}"
DESCARGAS=gdurdaneta/kubo-releases

git tag -a "$VERSION" -m "$MENSAJE"
git push origin "$VERSION"
gh workflow run build.yml --repo "$DESCARGAS" -f tag="$VERSION"

echo
echo "compilando: https://github.com/$DESCARGAS/actions"
echo "quedará en: https://github.com/$DESCARGAS/releases/tag/$VERSION"
