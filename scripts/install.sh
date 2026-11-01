#!/usr/bin/env sh
# Installs the Rune apps for this user: release binaries in ~/.local/bin, icons and launcher
# entries in ~/.local/share, and (with --dock) pins in GNOME's dock. Run from anywhere.
#
#   scripts/install.sh [--remote HOST] [--dock] [APP...]
#
# APP is mail, calendar, notes, phone or passwords; none means all of them. With --remote, HOST
# builds what's pushed to GitHub on this branch (it pulls first) and the binaries are copied
# back; without it, this machine builds the working tree as it is.
#
# Remote settings: RUNE_REMOTE_DIR, the checkout on HOST (default Projects/rune-apps, from its
# home), and RUNE_REMOTE_CARGO, how HOST runs cargo (default: through mise).
set -eu

cd "$(dirname "$0")/.."
remote=
dock=
apps=
while [ $# -gt 0 ]; do
    case "$1" in
        --remote) remote="${2:?--remote needs a host}"; shift 2 ;;
        --dock) dock=1; shift ;;
        -h | --help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) apps="$apps rune-${1#rune-}"; shift ;;
    esac
done
[ -n "$apps" ] || apps=$(for d in apps/*/; do basename "$d"; done)
for a in $apps; do
    [ -d "apps/$a" ] || { echo "No app called ${a#rune-} (apps/$a)" >&2; exit 1; }
done
packages=$(for a in $apps; do printf -- '-p %s ' "$a"; done)

if [ -n "$remote" ]; then
    dir="${RUNE_REMOTE_DIR:-Projects/rune-apps}"
    cargo="${RUNE_REMOTE_CARGO:-\$HOME/.local/bin/mise exec -- cargo}"
    branch=$(git rev-parse --abbrev-ref HEAD)
    git fetch -q origin "$branch" 2>/dev/null || true
    if [ -n "$(git status --porcelain)" ] || [ "$(git rev-parse HEAD)" != "$(git rev-parse "origin/$branch" 2>/dev/null)" ]; then
        echo "Note: $remote builds origin/$branch; changes here that aren't pushed aren't in it." >&2
    fi
    echo "Building on $remote…"
    ssh "$remote" "set -e; cd $dir && git fetch -q origin && git checkout -q $branch && git pull -q --ff-only origin $branch && $cargo build --release $packages"
    built=$(mktemp -d)
    trap 'rm -rf "$built"' EXIT
    for a in $apps; do
        scp -q "$remote:$dir/target/release/$a" "$built/$a"
    done
else
    cargo build --release $packages
    built=target/release
fi

bin="$HOME/.local/bin"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
desktop="$share/applications"
icons="$share/icons/hicolor"
mkdir -p "$bin" "$desktop" "$icons/scalable/apps"

for a in $apps; do
    install -m 755 "$built/$a" "$bin/$a"
    install -m 644 "apps/$a/assets/$a.svg" "$icons/scalable/apps/$a.svg"
    # The launcher entry runs the installed binary by its full path, so it works whether or not
    # ~/.local/bin is on the PATH the desktop session sees.
    sed "s|^Exec=$a|Exec=$bin/$a|" "apps/$a/assets/$a.desktop" > "$desktop/$a.desktop"
    chmod 644 "$desktop/$a.desktop"
    if [ -n "$dock" ] && command -v gsettings >/dev/null; then
        favs=$(gsettings get org.gnome.shell favorite-apps)
        case "$favs" in
            *"$a.desktop"*) ;;
            "@as []") gsettings set org.gnome.shell favorite-apps "['$a.desktop']" ;;
            *) gsettings set org.gnome.shell favorite-apps "$(printf '%s' "$favs" | sed "s/]$/, '$a.desktop']/")" ;;
        esac
    fi
    echo "Installed $bin/$a"
done

command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t -f "$icons" || true
command -v update-desktop-database >/dev/null && update-desktop-database -q "$desktop" || true
