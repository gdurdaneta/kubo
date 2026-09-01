#!/usr/bin/env bash
# Instala kubo para el usuario actual: binario, ícono y entrada de launcher.
#
# Todo va bajo ~/.local, sin sudo. Para desinstalar: ./instalar.sh --quitar
set -euo pipefail

BIN="$HOME/.local/bin"
APPS="$HOME/.local/share/applications"
ICONOS="$HOME/.local/share/icons/hicolor"
RAIZ="$(cd "$(dirname "$0")" && pwd)"

if [ "${1:-}" = "--quitar" ]; then
  rm -f "$BIN/kubo" "$APPS/kubo.desktop"
  find "$ICONOS" -name "kubo.png" -delete 2>/dev/null || true
  echo "kubo desinstalado"
  exit 0
fi

# El binario: el de dist/ si existe (compilado para glibc viejo), si no el local.
ORIGEN="$RAIZ/dist/kubo"
[ -x "$ORIGEN" ] || ORIGEN="$RAIZ/target/release/kubo"
[ -x "$ORIGEN" ] || { echo "no encuentro el binario; corré 'cargo build --release'" >&2; exit 1; }

mkdir -p "$BIN" "$APPS"
install -m755 "$ORIGEN" "$BIN/kubo"

# El ícono en cada tamaño del tema hicolor, para que el launcher elija el suyo.
for s in 16 32 64 128 256 512; do
  d="$ICONOS/${s}x${s}/apps"
  mkdir -p "$d"
  install -m644 "$RAIZ/assets/iconset/kubo-${s}.png" "$d/kubo.png"
done

# StartupWMClass tiene que coincidir con el app_id que setea la app, si no el
# launcher no asocia la ventana abierta con su entrada.
cat > "$APPS/kubo.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=kubo
GenericName=Cliente de Kubernetes
Comment=Explorar clusters de Kubernetes: recursos en vivo, logs, shell y port-forward
Exec=kubo
Icon=kubo
Terminal=false
Categories=Development;
Keywords=kubernetes;k8s;kubectl;cluster;devops;contenedores;
StartupWMClass=kubo
DESKTOP

# Refrescar las cachés, si las herramientas están.
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true
command -v gtk-update-icon-cache  >/dev/null && gtk-update-icon-cache -qtf "$ICONOS" 2>/dev/null || true

echo "binario:  $BIN/kubo"
echo "launcher: $APPS/kubo.desktop"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo; echo "ojo: $BIN no está en tu PATH" ;;
esac
