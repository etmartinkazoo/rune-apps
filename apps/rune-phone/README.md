# Rune Phone

Calls and texts from an Android phone on Linux, drawn with [Rune](https://github.com/anomalyco/rune),
with nothing to install on the phone: it uses the phone's own Bluetooth services.

- **Calls** over Hands-Free: dial (keypad, recents, contacts), answer or decline (in the window
  or from the desktop notification), mute, keypad tones for menus, hang up. PipeWire carries the
  audio through this computer's speakers and microphone.
- **Messages** over the Message Access Profile: conversations, reading, replying and new ones,
  with a notification when a text arrives. SMS and MMS, and the RCS chat messages a phone hands
  over as MMS (Pixels with Google Messages do); replies from here always go out as SMS.
- **Contacts and recent calls** over the Phonebook Access Profile.

## What it needs

- BlueZ with obexd, for messages and contacts: `bluez` plus `bluez-obex` on Arch,
  `bluez-obexd` on Fedora and Debian/Ubuntu. obexd is D-Bus activated on first use; to keep it
  running from login, `systemctl --user enable --now obex.service`.
- PipeWire 1.4 or later with its Bluetooth telephony service (`org.pipewire.Telephony`, from
  WirePlumber), for calls.
- The phone paired: open Rune Phone, choose **Look for phones** while the phone shows
  Settings › Connected devices › Pair new device, and confirm the code on both. When the phone
  asks, allow access to contacts, call history and messages.

## Running

```sh
scripts/install.sh phone          # build, install to ~/.local/bin, add to the launcher
scripts/install.sh phone --dock   # and pin it in GNOME's dock
```

Headless scenes: `rune-phone --scene demo|incoming|call|keypad|contact|setup|pair --snap out.png`.

## Tests

`cargo test` runs the parsers and, against a private `dbus-daemon` with stand-ins for PipeWire's
telephony service, obexd and the notification service, the whole worker: listing, reading and
sending texts, a text arriving, dialling, tones, hanging up, and answering a call.
