#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source_dir=${1:-/usr/share/figlet/fonts}
destination="$project_root/static/figlet"

fonts=(
    phm-minecraft.flf
    phm-lcdmatrix.flf
    phm-c64.flf
    phm-cga.flf
    phm-dos-square.flf
    phm-vga-square.flf
    phm-shinonome.flf
    phm-largetype.flf
    future.tlf
    phm-dos.flf
    phm-dosv.flf
    phm-hdos.flf
    phm-vga.flf
    smblock.tlf
    phm-beyondneo-mono.flf
    phm-slanted.flf
    abraxas.flf
)

for font in "${fonts[@]}"; do
    if [[ ! -f "$source_dir/$font" ]]; then
        echo "Missing FIGlet font: $source_dir/$font" >&2
        exit 1
    fi
done

mkdir -p "$destination"
for font in "${fonts[@]}"; do
    install -m 0644 "$source_dir/$font" "$destination/$font"
done

echo "Imported ${#fonts[@]} FIGlet fonts into $destination"
