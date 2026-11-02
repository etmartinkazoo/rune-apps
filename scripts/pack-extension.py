#!/usr/bin/env python3
"""Packs a Chromium extension folder into a signed CRX3 file and prints the extension's id.

    scripts/pack-extension.py DIR KEY.pem OUT.crx

KEY.pem is an RSA private key (made with openssl if it doesn't exist yet, readable only by
you); the id comes from it, so the same key keeps the same id across rebuilds. Signing uses
the openssl command, so nothing outside the standard library is needed.
"""

import hashlib
import io
import os
import struct
import subprocess
import sys
import tempfile
import zipfile


def varint(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


def field(number, data):
    """A length-delimited protobuf field."""
    return varint(number << 3 | 2) + varint(len(data)) + data


def main():
    folder, key, out = sys.argv[1:4]
    if not os.path.exists(key):
        os.makedirs(os.path.dirname(key) or ".", exist_ok=True)
        old = os.umask(0o077)
        try:
            subprocess.run(["openssl", "genrsa", "-out", key, "2048"], check=True, capture_output=True)
        finally:
            os.umask(old)
    public = subprocess.run(["openssl", "rsa", "-in", key, "-pubout", "-outform", "DER"], check=True, capture_output=True).stdout

    archive = io.BytesIO()
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
        for root, _, files in os.walk(folder):
            for name in sorted(files):
                path = os.path.join(root, name)
                z.write(path, os.path.relpath(path, folder))
    archive = archive.getvalue()

    crx_id = hashlib.sha256(public).digest()[:16]
    signed_data = field(1, crx_id)
    message = b"CRX3 SignedData\x00" + struct.pack("<I", len(signed_data)) + signed_data + archive
    with tempfile.NamedTemporaryFile() as f:
        f.write(message)
        f.flush()
        signature = subprocess.run(["openssl", "dgst", "-sha256", "-sign", key, f.name], check=True, capture_output=True).stdout

    proof = field(1, public) + field(2, signature)
    header = field(2, proof) + field(10000, signed_data)
    with open(out, "wb") as f:
        f.write(b"Cr24" + struct.pack("<II", 3, len(header)) + header + archive)

    # The id: the first 16 bytes of the key's hash, each nibble as a letter a-p.
    print("".join(chr(ord("a") + int(c, 16)) for c in crx_id.hex()))


if __name__ == "__main__":
    main()
