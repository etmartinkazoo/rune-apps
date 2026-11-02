#!/usr/bin/env sh
# Installs the Rune apps for this user: release binaries in ~/.local/bin, icons and launcher
# entries in ~/.local/share, and (with --dock) pins in GNOME's dock. Run from anywhere.
#
#   scripts/install.sh [--remote HOST] [--dock] [--no-deps] [APP...]
#
# APP is mail, calendar, notes, phone or passwords; none means all of them. With --remote, HOST
# builds what's pushed to GitHub on this branch (it pulls first) and the binaries are copied
# back; without it, this machine builds the working tree as it is.
#
# Remote settings: RUNE_REMOTE_DIR, the checkout on HOST (default Projects/rune-apps, from its
# home), and RUNE_REMOTE_CARGO, how HOST runs cargo (default: through mise).
#
# On Fedora 45 it first installs (with dnf, through sudo) the packages missing for building and
# for running the apps chosen, and turns on what they need running; --no-deps skips that.
set -eu

cd "$(dirname "$0")/.."
remote=
dock=
deps=1
apps=
while [ $# -gt 0 ]; do
    case "$1" in
        --remote) remote="${2:?--remote needs a host}"; shift 2 ;;
        --dock) dock=1; shift ;;
        --no-deps) deps=; shift ;;
        -h | --help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) apps="$apps rune-${1#rune-}"; shift ;;
    esac
done
[ -n "$apps" ] || apps=$(for d in apps/*/; do basename "$d"; done)
for a in $apps; do
    [ -d "apps/$a" ] || { echo "No app called ${a#rune-} (apps/$a)" >&2; exit 1; }
done

# Fedora 45 packages: what building takes (here, unless HOST builds), and what each app needs to run.
fedora_packages() {
    [ -n "$remote" ] || echo libxkbcommon-devel pkgconf-pkg-config openssl-devel perl perl-IPC-Cmd perl-FindBin make gcc
    # Drawing (Vulkan), the keyboard, and the desktop portals (light or dark, file dialogs).
    echo mesa-vulkan-drivers libxkbcommon xdg-desktop-portal
    for a in $apps; do
        case "$a" in
            rune-mail) echo fontconfig ;;
            # BlueZ for pairing, obexd for messages and contacts, PipeWire and WirePlumber for calls.
            rune-phone) echo bluez bluez-obexd pipewire wireplumber ;;
        esac
    done
}

# Mail, Calendar and Passwords keep their secrets with the desktop's Secret Service: oo7 (Fedora
# 45's own) or GNOME Keyring. Either will do; with neither, oo7.
needs_secrets() {
    for a in $apps; do
        case "$a" in rune-mail | rune-calendar | rune-passwords) return 0 ;; esac
    done
    return 1
}

if [ -n "$deps" ] && [ -r /etc/os-release ] && . /etc/os-release && [ "${ID:-}" = fedora ] && [ "${VERSION_ID:-}" = 45 ]; then
    missing=
    for p in $(fedora_packages | tr ' ' '\n' | sort -u); do
        rpm -q "$p" >/dev/null 2>&1 || missing="$missing $p"
    done
    if needs_secrets && ! rpm -q oo7-daemon >/dev/null 2>&1 && ! rpm -q gnome-keyring >/dev/null 2>&1; then
        missing="$missing oo7-daemon"
    fi
    if [ -n "$missing" ]; then
        echo "Installing Fedora packages:$missing"
        sudo dnf install -y $missing
    fi
    case " $apps " in
        *" rune-phone "*)
            # obexd runs as a user service; the phone's messages and contacts need it running.
            systemctl --user is-active -q obex.service 2>/dev/null || systemctl --user enable --now obex.service || true ;;
    esac
fi

# Always the whole workspace, even for one app: cargo turns on dependencies' features per build,
# so building one app alone would rebuild Rune with other features and throw the cached build away.

if [ -n "$remote" ]; then
    dir="${RUNE_REMOTE_DIR:-Projects/rune-apps}"
    cargo="${RUNE_REMOTE_CARGO:-\$HOME/.local/bin/mise exec -- cargo}"
    branch=$(git rev-parse --abbrev-ref HEAD)
    git fetch -q origin "$branch" 2>/dev/null || true
    if [ -n "$(git status --porcelain)" ] || [ "$(git rev-parse HEAD)" != "$(git rev-parse "origin/$branch" 2>/dev/null)" ]; then
        echo "Note: $remote builds origin/$branch; changes here that aren't pushed aren't in it." >&2
    fi
    echo "Building on $remote…"
    ssh "$remote" "set -e; cd $dir && git fetch -q origin && git checkout -q $branch && git pull -q --ff-only origin $branch && $cargo build --release --workspace"
    built=$(mktemp -d)
    trap 'rm -rf "$built"' EXIT
    for a in $apps; do
        scp -q "$remote:$dir/target/release/$a" "$built/$a"
    done
else
    cargo build --release --workspace
    built=target/release
fi

# Writes a root-owned file from stdin, through sudo, only when it would change.
root_file() {
    tmp=$(mktemp)
    cat > "$tmp"
    if ! cmp -s "$tmp" "$1"; then
        sudo install -D -m 644 "$tmp" "$1"
    fi
    rm -f "$tmp"
}

# Rune Passwords' Chromium extension, installed and pinned with nothing to do in the browser:
# - packed and signed with a key made once for this computer (its id comes from the key);
# - the native-messaging host it reaches the window through (this binary), for that id only;
# - Chromium's external-extension file, which installs the package at the browser's next start
#   (bump "version" in its manifest.json for a change to reach an installed copy);
# - the browser policy (scripts/chromium-policy.json), with the extension pinned.
install_extension() {
    data="$share/rune-passwords"
    crx="/usr/share/rune-passwords/rune-passwords.crx"
    id=$(python3 -I scripts/pack-extension.py apps/rune-passwords/extension "$data/extension.pem" "$data/rune-passwords.crx")
    version=$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["version"])' apps/rune-passwords/extension/manifest.json)
    for browser in chromium google-chrome; do
        hosts="${XDG_CONFIG_HOME:-$HOME/.config}/$browser/NativeMessagingHosts"
        mkdir -p "$hosts"
        cat > "$hosts/io.brem.rune_passwords.json" <<EOF
{
  "name": "io.brem.rune_passwords",
  "description": "Rune Passwords",
  "path": "$bin/rune-passwords",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://$id/"]
}
EOF
    done
    command -v chromium-browser >/dev/null || return 0
    root_file "$crx" < "$data/rune-passwords.crx"
    printf '{ "external_crx": "%s", "external_version": "%s" }\n' "$crx" "$version" | root_file "/usr/share/chromium/extensions/$id.json"
    python3 -I -c '
import json, sys
policy = json.load(open(sys.argv[1]))
policy.setdefault("ExtensionSettings", {})[sys.argv[2]] = {"toolbar_pin": "force_pinned"}
print(json.dumps(policy, indent=2))' scripts/chromium-policy.json "$id" | root_file /etc/chromium/policies/managed/rune.json
    # One policy file only: Chromium doesn't merge ExtensionSettings across files.
    [ ! -e /etc/chromium/policies/managed/quiet.json ] || sudo rm -f /etc/chromium/policies/managed/quiet.json
    echo "Chromium: Rune Passwords extension ($id) and policy installed; restart Chromium to pick them up"
}

bin="$HOME/.local/bin"

# macOS: each app as a bundle in /Applications (~/Applications if that isn't writable), with its
# icon made from the SVG, and ~/.local/bin/APP a link to the binary inside.
if [ "$(uname)" = Darwin ]; then
    apps_dir="${RUNE_APPLICATIONS:-/Applications}"
    [ -w "$apps_dir" ] || apps_dir="$HOME/Applications"
    mkdir -p "$bin" "$apps_dir"
    work=$(mktemp -d)
    trap 'rm -rf "$work"; [ -z "$remote" ] || rm -rf "$built"' EXIT
    for a in $apps; do
        name=$(sed -n 's/^Name=//p' "apps/$a/assets/$a.desktop" | head -n 1)
        app="$apps_dir/$name.app"
        # The SVG's tile fills 448 of its 512; macOS icons keep theirs to about 80% of the canvas.
        sed 's|viewBox="0 0 512 512"|viewBox="-22 -22 556 556"|' "apps/$a/assets/$a.svg" > "$work/$a.svg"
        set_dir="$work/$a.iconset"
        mkdir -p "$set_dir"
        for s in 16 32 128 256 512; do
            sips -s format png -z $s $s "$work/$a.svg" --out "$set_dir/icon_${s}x${s}.png" >/dev/null
            sips -s format png -z $((s * 2)) $((s * 2)) "$work/$a.svg" --out "$set_dir/icon_${s}x${s}@2x.png" >/dev/null
        done
        rm -rf "$app"
        mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
        iconutil -c icns "$set_dir" -o "$app/Contents/Resources/$a.icns"
        install -m 755 "$built/$a" "$app/Contents/MacOS/$a"
        version=$(sed -n 's/^version = "\(.*\)"/\1/p' "apps/$a/Cargo.toml" | head -n 1)
        # Why the app uses what macOS asks permission for; without it, macOS ends the app.
        usage=
        if [ "$a" = rune-phone ]; then
            usage='    <key>NSBluetoothAlwaysUsageDescription</key><string>Rune Phone reaches your phone over Bluetooth for its texts, contacts and recent calls.</string>'
        fi
        cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$name</string>
    <key>CFBundleDisplayName</key><string>$name</string>
    <key>CFBundleIdentifier</key><string>io.brem.$a</string>
    <key>CFBundleExecutable</key><string>$a</string>
    <key>CFBundleIconFile</key><string>$a</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleVersion</key><string>${version:-0.1.0}</string>
    <key>CFBundleShortVersionString</key><string>${version:-0.1.0}</string>
    <key>NSHighResolutionCapable</key><true/>
$usage
</dict>
</plist>
EOF
        # A fresh bundle at the same path can keep the old (blank) icon in Finder and the Dock
        # until Launch Services looks at it again.
        touch "$app"
        /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app" 2>/dev/null || true
        ln -sf "$app/Contents/MacOS/$a" "$bin/$a"
        echo "Installed $app"
    done
    exit 0
fi

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
    if [ "$a" = rune-passwords ]; then
        install_extension
    fi
    echo "Installed $bin/$a"
done

command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t -f "$icons" || true
command -v update-desktop-database >/dev/null && update-desktop-database -q "$desktop" || true
