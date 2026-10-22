#!/usr/bin/env bash
# Installs AWorld for the current user and puts a shortcut on the desktop (GNOME and other freedesktop
# desktops) and in the applications menu.
#   ./install.sh              build, install, create the shortcuts
#   ./install.sh --uninstall  remove them again
set -euo pipefail
cd "$(dirname "$0")"
here="$(pwd)"

data="${XDG_DATA_HOME:-$HOME/.local/share}"
home_dir="$data/aworld"
apps="$data/applications"
desktop_dir="$( (command -v xdg-user-dir >/dev/null 2>&1 && xdg-user-dir DESKTOP) || echo "$HOME/Desktop")"
file_name="aworld.desktop"

if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$apps/$file_name" "$desktop_dir/$file_name"
  rm -rf "$home_dir/aworld" "$home_dir/icon.png"
  echo "Raccourcis et programme retirés (tes mondes sauvegardés restent dans $home_dir/saves)."
  exit 0
fi

./build.sh app
mkdir -p "$home_dir" "$apps"
cp target/release/aworld "$home_dir/aworld"
cp assets/icon.png "$home_dir/icon.png"

make_entry() {
  cat > "$1" <<DESKTOP
[Desktop Entry]
Type=Application
Name=AWorld
Comment=Bac à sable éducatif : sable, eau, chimie, blocs, mécanique et voitures
Exec="$home_dir/aworld"
Path=$home_dir
Icon=$home_dir/icon.png
Terminal=false
Categories=Game;Education;Simulation;
StartupWMClass=AWorld
DESKTOP
  chmod +x "$1"
}

make_entry "$apps/$file_name"
if [ -d "$desktop_dir" ]; then
  make_entry "$desktop_dir/$file_name"
  # GNOME only lets a desktop file be launched once it is marked as trusted.
  command -v gio >/dev/null 2>&1 && gio set "$desktop_dir/$file_name" metadata::trusted true 2>/dev/null || true
else
  echo "Pas de dossier Bureau trouvé ($desktop_dir) : seul le menu des applications a été mis à jour."
fi
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" 2>/dev/null || true

echo "OK : AWorld installé dans $home_dir"
echo "Raccourci : $desktop_dir/$file_name (et dans le menu des applications)."
echo "Si le raccourci du bureau reste grisé : clic droit > « Autoriser le lancement »."
