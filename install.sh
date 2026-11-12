#!/usr/bin/env bash
# Pasang ytstream-ingame-overlay sebagai aplikasi desktop:
#   - cek/pasang dependensi sistem (gtk4-layer-shell, v4l-utils)
#   - build release
#   - pasang binary, entri menu KDE, dan ikon
#   - jalankan aplikasinya setelah selesai
#
# Pakai: ./install.sh              (pasang + langsung jalankan)
#        ./install.sh --no-run     (pasang saja, tanpa menjalankan)
#        ./install.sh --no-deps    (lewati pemasangan dependensi sistem)
#        ./install.sh --uninstall  (hapus binary, entri menu, dan ikon)
set -euo pipefail

SKIP_DEPS=0
LAUNCH=1
UNINSTALL=0
for arg in "$@"; do
    case "$arg" in
        --no-deps) SKIP_DEPS=1 ;;
        --no-run) LAUNCH=0 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help)
            awk 'NR > 1 { if (/^#/) { sub(/^# ?/, ""); print; next } exit }' "$0"
            exit 0
            ;;
        *)
            echo "Opsi tidak dikenal: $arg" >&2
            exit 1
            ;;
    esac
done

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cd "$SCRIPT_DIR"

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }
warn() { printf '\033[33m%s\033[0m\n' "$*" >&2; }

BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
DESKTOP_DIR="$DATA_DIR/applications"
ICON_DIR="$DATA_DIR/icons/hicolor/scalable/apps"

# --- 0. Uninstall -------------------------------------------------------------
if [ "$UNINSTALL" -eq 1 ]; then
    say "Menghapus ytstream-ingame-overlay…"

    # Hentikan aplikasi yang sedang berjalan supaya tidak ada proses yang
    # memakai file yang dihapus.
    if [ -x "$BIN_DIR/ytstream-ingame-overlay" ] && [ -S "${XDG_RUNTIME_DIR:-/tmp}/ytstream-ingame-overlay.sock" ]; then
        # Exit code `ctl quit` bisa error karena koneksi terputus saat app keluar;
        # keberhasilannya dicek lewat pgrep di bawah.
        "$BIN_DIR/ytstream-ingame-overlay" ctl quit >/dev/null 2>&1 || true
        sleep 1
    fi

    removed=0
    # Nama lama (sebelum rename) ikut dibersihkan kalau masih ada.
    for f in \
        "$BIN_DIR/ytstream-ingame-overlay" \
        "$DESKTOP_DIR/ytstream-ingame-overlay.desktop" \
        "$ICON_DIR/ytstream-ingame-overlay.svg" \
        "$BIN_DIR/ytchat-overlay" \
        "$DESKTOP_DIR/ytchat-overlay.desktop" \
        "$ICON_DIR/ytchat-overlay.svg"; do
        if [ -e "$f" ]; then
            rm -f "$f"
            echo "  dihapus: $f"
            removed=1
        fi
    done

    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q "$DATA_DIR/icons/hicolor" >/dev/null 2>&1 || true
    fi

    [ "$removed" -eq 1 ] || warn "Tidak ada file instalasi yang ditemukan."
    # Nama biner lebih dari 15 karakter, jadi `pgrep -x` tidak bisa dipakai;
    # pola `[/]` menghindari ikut mencocokkan baris perintah skrip ini sendiri.
    if pgrep -f "[/]ytstream-ingame-overlay" >/dev/null 2>&1; then
        warn "Masih ada proses ytstream-ingame-overlay; tutup panelnya atau jalankan: pkill -f ytstream-ingame-overlay"
    fi
    say "Selesai."
    echo "Config dan setelanmu dibiarkan di: ${XDG_CONFIG_HOME:-$HOME/.config}/ytstream-ingame-overlay/"
    echo "Hapus folder itu kalau ingin membuang link, API key, dan setelan juga."
    exit 0
fi

# --- 1. Dependensi sistem -----------------------------------------------------
# gtk4-layer-shell untuk jendela overlay, libv4l2 (paket v4l-utils) untuk webcam.
MISSING=()
pkg-config --exists gtk4-layer-shell 2>/dev/null || MISSING+=(gtk4-layer-shell)
pkg-config --exists libv4l2 2>/dev/null || MISSING+=(v4l-utils)

if [ "${#MISSING[@]}" -eq 0 ]; then
    echo "✔ Dependensi sistem sudah terpasang (gtk4-layer-shell, libv4l2)."
elif [ "$SKIP_DEPS" -eq 1 ]; then
    warn "Dependensi belum lengkap (${MISSING[*]}), dilewati karena --no-deps."
elif command -v pacman >/dev/null 2>&1; then
    if [ ! -t 0 ]; then
        warn "Jalankan skrip ini dari terminal supaya bisa memasang dependensi (sudo)."
        warn "Atau pasang dulu: sudo pacman -S ${MISSING[*]}"
        exit 1
    fi
    say "Memasang dependensi sistem: ${MISSING[*]} (butuh password sudo)"
    sudo pacman -S --needed "${MISSING[@]}"
else
    warn "Dependensi belum terpasang: ${MISSING[*]}. Pasang lewat package manager distro-mu dulu."
fi

# --- 2. Build ----------------------------------------------------------------
if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo tidak ditemukan. Pasang Rust dulu, mis.: sudo pacman -S rust" >&2
    exit 1
fi

say "Membangun versi release (sekali saja, beberapa menit)…"
cargo build --release

# --- 3. Pasang file ----------------------------------------------------------
# Pemasangan dengan nama lama dihentikan dan dibuang, supaya di menu tidak ada
# dua entri untuk aplikasi yang sama.
if pgrep -x ytchat-overlay >/dev/null 2>&1; then
    pkill -x ytchat-overlay 2>/dev/null || true
    echo "  aplikasi versi lama (ytchat-overlay) dihentikan"
fi
for f in \
    "$BIN_DIR/ytchat-overlay" \
    "$DESKTOP_DIR/ytchat-overlay.desktop" \
    "$ICON_DIR/ytchat-overlay.svg"; do
    if [ -e "$f" ]; then
        rm -f "$f"
        echo "  dihapus (nama lama): $f"
    fi
done

install -d "$BIN_DIR" "$DESKTOP_DIR" "$ICON_DIR"
install -m755 target/release/ytstream-ingame-overlay "$BIN_DIR/ytstream-ingame-overlay"
install -m644 assets/ytstream-ingame-overlay.svg "$ICON_DIR/ytstream-ingame-overlay.svg"

# Exec memakai path absolut supaya tidak bergantung pada PATH sesi KDE.
sed "s|^Exec=.*|Exec=$BIN_DIR/ytstream-ingame-overlay|" \
    assets/ytstream-ingame-overlay.desktop >"$DESKTOP_DIR/ytstream-ingame-overlay.desktop"
chmod 644 "$DESKTOP_DIR/ytstream-ingame-overlay.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q "$DATA_DIR/icons/hicolor" >/dev/null 2>&1 || true
fi

say "Selesai — aplikasi sudah terpasang."
echo "Program : $BIN_DIR/ytstream-ingame-overlay"
echo "Menu    : cari \"YTStream In-Game Overlay\" di menu aplikasi KDE"
if command -v kbuildsycoca6 >/dev/null 2>&1; then
    echo "Kalau ikonnya belum muncul di menu: kbuildsycoca6"
fi

# --- 4. Jalankan --------------------------------------------------------------
if [ "$LAUNCH" -eq 1 ]; then
    if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${DISPLAY:-}" ]; then
        warn "Tidak ada sesi grafis aktif (WAYLAND_DISPLAY/DISPLAY kosong); aplikasi tidak dijalankan."
        exit 0
    fi
    say "Menjalankan aplikasi…"
    # Dilepas dari terminal supaya tetap hidup walau terminal ditutup.
    setsid "$BIN_DIR/ytstream-ingame-overlay" >/dev/null 2>&1 </dev/null &
    disown 2>/dev/null || true
    echo "Panel YTStream In-Game Overlay akan terbuka. Kalau sudah berjalan, panelnya dimunculkan lagi."
fi
