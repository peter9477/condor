#!/bin/sh
# Build the Windows release and install it as C:\Users\<you>\.local\bin\condor.exe.
# A running condor.exe can't be overwritten but can be renamed, so the old one is moved
# aside to condor.exe.old (removed on the next deploy) and open windows keep running.
set -e
cd "$(dirname "$0")/.."
cargo xwin build --release --target x86_64-pc-windows-msvc
TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
EXE="$TARGET_DIR/x86_64-pc-windows-msvc/release/condor.exe"
WINPROFILE=$(/mnt/c/Windows/System32/cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')
[ -n "$WINPROFILE" ] || { echo "Can't find the Windows user profile" >&2; exit 1; }
WINHOME=$(wslpath "$WINPROFILE")
DEST="$WINHOME/.local/bin"
mkdir -p "$DEST"
rm -f "$DEST/condor.exe.old" 2>/dev/null || true
[ -f "$DEST/condor.exe" ] && mv "$DEST/condor.exe" "$DEST/condor.exe.old"
cp "$EXE" "$DEST/condor.exe"
echo "Installed $DEST/condor.exe"
