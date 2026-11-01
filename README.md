# Rune apps

Desktop apps drawn with [Rune](https://github.com/anomalyco/rune), in one Cargo workspace so
Rune and the other dependencies build once for all of them.

| App | What it is |
|---|---|
| [Rune Mail](apps/rune-mail) | Mail, laid out like Apple Mail: Fastmail over JMAP, Gmail and others over IMAP |
| [Rune Calendar](apps/rune-calendar) | Day, Week and Month views of your CalDAV calendars |
| [Rune Notes](apps/rune-notes) | Notes as plain Markdown files in real folders |
| [Rune Phone](apps/rune-phone) | Calls and texts from an Android phone over Bluetooth |
| [Rune Passwords](apps/rune-passwords) | A Vaultwarden / Bitwarden vault, laid out like Apple Passwords |

## Building

Linux needs the XKB and Wayland development packages (`libxkbcommon-dev libwayland-dev
pkg-config` on Ubuntu, `libxkbcommon-devel wayland-devel pkgconf` on Fedora).

```sh
cargo build --release                 # every app, into target/release
cargo run -p rune-passwords           # one app
cargo test --workspace
```

## Installing

```sh
scripts/install.sh                    # build and install every app, with icons and launchers
scripts/install.sh mail passwords     # just these
scripts/install.sh --dock             # and pin them in GNOME's dock
scripts/install.sh --remote beebox    # build on another machine over SSH and install here
```

With `--remote`, the other machine pulls this branch from GitHub and builds it, so push first.
It needs a checkout at `~/Projects/rune-apps` and cargo through mise; `RUNE_REMOTE_DIR` and
`RUNE_REMOTE_CARGO` change those. Its glibc must be no newer than this machine's for the
binaries to run here.

Every app has headless scenes for screenshots, e.g. `rune-passwords --scene demo --snap out.png`.

## Developing on Beebox

Beebox (Ubuntu, over Tailscale) is where these are developed and built; the checkout is
`~/Projects/rune-apps`, with Rust from mise (`mise use -g rust@latest`). Its system packages:

```sh
sudo apt-get install -y libxkbcommon-dev libwayland-dev pkg-config mold clang mesa-vulkan-drivers
```

Long builds run detached, so a dropped SSH connection doesn't stop them:

```sh
nohup cargo build --release --workspace > ~/rune-build.log 2>&1 &
nohup cargo test --workspace > ~/rune-test.log 2>&1 &
tail -f ~/rune-test.log
```

Then, on the laptop, `scripts/install.sh --remote beebox` installs what Beebox built from the
pushed branch.
