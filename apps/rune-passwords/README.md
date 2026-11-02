# Rune Passwords

A password manager for your [Vaultwarden](https://github.com/dani-garcia/vaultwarden) (or
Bitwarden) vault, laid out like Apple Passwords and drawn with
[Rune](https://github.com/anomalyco/rune).

- Categories down the side (All, Passkeys, Codes, Notes, Security, Deleted), with your folders
  and the organisations items are shared through.
- Click a field to copy it; reveal a password with the eye; websites open in the browser.
- Verification codes (TOTP, including `otpauth://` links and Steam) count down live.
- Sites' own icons, fetched through your server's icon service, with a letter tile until one
  arrives (or when a site has none).
- Swipe left with two fingers on a row for Copy User Name, Copy Password and Delete (Recover
  and Delete Now in Deleted); swipe all the way to delete. Deleting always asks first.
- Create, edit, delete and recover items. A changed password goes into the item's history, as
  the official apps keep it, and "Suggest a strong password" makes Apple-style ones.
- Security: weak passwords (zxcvbn) and passwords used by more than one item.
- Opens offline from the last sync, and locks after a set time without input.
- More than one account, on one server or several: the account at the bottom of the sidebar
  switches between them or adds another. Each has its own master password and cached vault.

## Security

Everything is encrypted and decrypted on this computer with Bitwarden's scheme (PBKDF2 or
Argon2id, AES-256-CBC with HMAC-SHA256, RSA-OAEP for shared items); the server only ever sees
ciphertext and a hash of the master key. The master password is never stored. What's kept
between runs: the server address, the encrypted user key and the last sync (all ciphertext) in
files only you can read (one set per account), and each session's refresh token in the system keyring. Site icons
are kept unencrypted in `~/.cache/rune-passwords/icons`, named by site, so that folder shows
which sites the vault holds; delete it to clear them.

## Running

```sh
scripts/install.sh passwords          # build, install to ~/.local/bin, add to the launcher
scripts/install.sh passwords --dock   # and pin it in GNOME's dock
```

Headless scenes for screenshots: `rune-passwords --scene demo|edit|new|security|deleted|swipe|confirm|settings|accounts|folders|signin|unlock --snap out.png`.

## Browser extension

`extension/` fills logins into Chromium from every account unlocked in the running app (the
app keeps each account you unlock open until you lock). A login field gets a menu under it with
the site's logins (a verification-code field, their codes), drawn in a frame the page can't
read; the toolbar icon's badge counts the site's logins; its popup lists them, with the accounts
to choose from at the top right, a click to fill, and **Code** to copy the verification code. `scripts/install.sh passwords` sets it all up, with nothing to do
in the browser: it packs and signs the extension with a key made once for this computer
(`~/.local/share/rune-passwords/extension.pem`), registers the native-messaging host it talks
through (this binary, started by the browser) for that extension only, and, through sudo,
installs the package as an external extension (`/usr/share/chromium/extensions/`) and the
browser policy in `scripts/chromium-policy.json` (`/etc/chromium/policies/managed/rune.json`),
which pins it. Restart Chromium to pick it up; bump `version` in `extension/manifest.json` for a
change to reach an installed copy. The app answers it over a socket in `$XDG_RUNTIME_DIR` that
only this user can open, only for the page's own site, and not at all while the vault is locked.

## Tests

`cargo test` runs everything that needs no server. Against a throwaway Vaultwarden that allows
sign-ups (never your own):

```sh
RUNE_PASSWORDS_TEST_SERVER=http://127.0.0.1:8222 cargo test -- --ignored --test-threads=1
```
