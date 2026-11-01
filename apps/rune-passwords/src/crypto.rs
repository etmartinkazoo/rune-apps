//! Bitwarden's vault encryption, as Vaultwarden stores it: everything is encrypted on this
//! machine before it leaves, and the server holds only ciphertext.
//!
//! - The master key comes from the master password and the email address through the account's
//!   KDF: PBKDF2-SHA256 (salted with the address) or Argon2id (salted with its SHA-256).
//! - The server is shown a hash of the master key, never the key: PBKDF2 once more, salted
//!   with the password.
//! - The master key, stretched with HKDF into an encryption and a MAC key, unlocks the user
//!   key; the user key (and an organisation's key, or an item's own key) encrypts the items.
//! - Each encrypted value is an "EncString": `2.iv|data|mac`, AES-256-CBC with HMAC-SHA256
//!   over the IV and ciphertext. Organisation keys come RSA-OAEP encrypted (`4.data`) to the
//!   account's key pair.

use aes::Aes256;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub type Result<T> = std::result::Result<T, String>;

/// How the account turns its master password into a key, as the server reports it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Kdf {
    Pbkdf2 { iterations: u32 },
    Argon2id { iterations: u32, memory_mib: u32, parallelism: u32 },
}

impl Kdf {
    /// From prelogin's `kdf` (0 PBKDF2, 1 Argon2id) and its parameters.
    pub fn from_wire(kind: u64, iterations: u64, memory: Option<u64>, parallelism: Option<u64>) -> Result<Kdf> {
        let n = |v: u64| u32::try_from(v).map_err(|_| "The server sent a key setting out of range".to_owned());
        match kind {
            0 => Ok(Kdf::Pbkdf2 { iterations: n(iterations)? }),
            1 => Ok(Kdf::Argon2id {
                iterations: n(iterations)?,
                memory_mib: n(memory.unwrap_or(64))?,
                parallelism: n(parallelism.unwrap_or(4))?,
            }),
            k => Err(format!("This account uses a key setting this app doesn't know ({k})")),
        }
    }
}

/// 32 bytes that wipe themselves when dropped.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Key32(pub [u8; 32]);

/// The master key: from the master password, the address and the KDF.
pub fn master_key(password: &str, email: &str, kdf: Kdf) -> Result<Key32> {
    let email = email.trim().to_lowercase();
    let mut out = [0u8; 32];
    match kdf {
        Kdf::Pbkdf2 { iterations } => {
            pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), email.as_bytes(), iterations.max(1), &mut out);
        }
        Kdf::Argon2id { iterations, memory_mib, parallelism } => {
            let salt = Sha256::digest(email.as_bytes());
            let params = argon2::Params::new(memory_mib.saturating_mul(1024), iterations, parallelism, Some(32))
                .map_err(|e| format!("Argon2 settings: {e}"))?;
            argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
                .hash_password_into(password.as_bytes(), &salt, &mut out)
                .map_err(|e| format!("Argon2: {e}"))?;
        }
    }
    Ok(Key32(out))
}

/// What the server checks a sign-in against: the master key hashed once more with the
/// password, never the key itself.
pub fn password_hash(master: &Key32, password: &str) -> String {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(&master.0, password.as_bytes(), 1, &mut out);
    B64.encode(out)
}

/// A symmetric key: AES-256 for the data, HMAC-SHA256 for its integrity.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SymKey {
    pub enc: [u8; 32],
    pub mac: Option<[u8; 32]>,
}

impl SymKey {
    /// From 64 bytes (encryption key, then MAC key) or a bare 32-byte encryption key.
    pub fn from_bytes(b: &[u8]) -> Result<SymKey> {
        match b.len() {
            64 => {
                let mut enc = [0u8; 32];
                let mut mac = [0u8; 32];
                enc.copy_from_slice(&b[..32]);
                mac.copy_from_slice(&b[32..]);
                Ok(SymKey { enc, mac: Some(mac) })
            }
            32 => {
                let mut enc = [0u8; 32];
                enc.copy_from_slice(b);
                Ok(SymKey { enc, mac: None })
            }
            n => Err(format!("A key of {n} bytes isn't one")),
        }
    }

    /// The master key stretched into an encryption and a MAC key (HKDF-Expand, SHA-256).
    pub fn stretch(master: &Key32) -> SymKey {
        let hk = hkdf::Hkdf::<Sha256>::from_prk(&master.0).expect("32 bytes is a valid PRK");
        let mut enc = [0u8; 32];
        let mut mac = [0u8; 32];
        hk.expand(b"enc", &mut enc).expect("32 bytes is a valid length");
        hk.expand(b"mac", &mut mac).expect("32 bytes is a valid length");
        SymKey { enc, mac: Some(mac) }
    }

    /// A new random key, as the vault makes for an account or an item.
    #[cfg(test)]
    pub fn random() -> SymKey {
        let mut b = [0u8; 64];
        getrandom::fill(&mut b).expect("the system's random source");
        let k = SymKey::from_bytes(&b).expect("64 bytes");
        b.zeroize();
        k
    }

    /// The 64 bytes, for encrypting this key under another.
    #[cfg(test)]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = self.enc.to_vec();
        v.extend_from_slice(self.mac.as_ref().map_or(&[][..], |m| &m[..]));
        v
    }

    /// Encrypts `plain` as a type-2 EncString.
    pub fn encrypt(&self, plain: &[u8]) -> String {
        let mut iv = [0u8; 16];
        getrandom::fill(&mut iv).expect("the system's random source");
        let ct = cbc::Encryptor::<Aes256>::new(&self.enc.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(plain);
        let mac_key = self.mac.expect("the vault's keys carry a MAC key");
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&mac_key).expect("any key length");
        m.update(&iv);
        m.update(&ct);
        let mac = m.finalize().into_bytes();
        format!("2.{}|{}|{}", B64.encode(iv), B64.encode(&ct), B64.encode(mac))
    }

    pub fn encrypt_str(&self, plain: &str) -> String {
        self.encrypt(plain.as_bytes())
    }

    /// Decrypts a type-0 or type-2 EncString, checking its MAC first.
    pub fn decrypt(&self, enc: &str) -> Result<Vec<u8>> {
        let (kind, rest) = enc.split_once('.').ok_or("Not an encrypted value")?;
        let parts: Vec<&str> = rest.split('|').collect();
        let b = |s: &str| B64.decode(s).map_err(|_| "Encrypted value isn't base64".to_owned());
        let (iv, ct) = match (kind, parts.as_slice()) {
            ("2", [iv, ct, mac]) => {
                let (iv, ct, mac) = (b(iv)?, b(ct)?, b(mac)?);
                let key = self.mac.ok_or("A key without a MAC key can't check this value")?;
                let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&key).expect("any key length");
                m.update(&iv);
                m.update(&ct);
                m.verify_slice(&mac).map_err(|_| "Wrong key, or the value was altered".to_owned())?;
                (iv, ct)
            }
            ("0", [iv, ct]) => (b(iv)?, b(ct)?),
            (k, _) => return Err(format!("Encrypted values of type {k} aren't supported")),
        };
        let iv: [u8; 16] = iv.try_into().map_err(|_| "Bad IV".to_owned())?;
        cbc::Decryptor::<Aes256>::new(&self.enc.into(), &iv.into())
            .decrypt_padded_vec_mut::<Pkcs7>(&ct)
            .map_err(|_| "Wrong key, or the value was altered".to_owned())
    }

    pub fn decrypt_str(&self, enc: &str) -> Result<String> {
        String::from_utf8(self.decrypt(enc)?).map_err(|_| "Decrypted text isn't UTF-8".to_owned())
    }

    /// A key encrypted under this one (the user key, an item's key).
    pub fn decrypt_key(&self, enc: &str) -> Result<SymKey> {
        let mut b = self.decrypt(enc)?;
        let k = SymKey::from_bytes(&b);
        b.zeroize();
        k
    }
}

/// The account's RSA private key, which opens organisation keys.
pub struct PrivateKey(rsa::RsaPrivateKey);

impl PrivateKey {
    /// From the profile's encrypted PKCS#8 key, opened with the user key.
    pub fn open(enc: &str, user: &SymKey) -> Result<PrivateKey> {
        use rsa::pkcs8::DecodePrivateKey;
        let mut der = user.decrypt(enc)?;
        let key = rsa::RsaPrivateKey::from_pkcs8_der(&der).map_err(|e| format!("Private key: {e}"));
        der.zeroize();
        key.map(PrivateKey)
    }

    /// Decrypts an RSA EncString (`4.` OAEP-SHA1, `3.` OAEP-SHA256) holding a symmetric key.
    pub fn decrypt_key(&self, enc: &str) -> Result<SymKey> {
        let (kind, data) = enc.split_once('.').ok_or("Not an encrypted key")?;
        let data = B64.decode(data.split('|').next().unwrap_or_default()).map_err(|_| "Key isn't base64".to_owned())?;
        let mut plain = match kind {
            "4" | "6" => self.0.decrypt(rsa::Oaep::new::<sha1::Sha1>(), &data),
            "3" | "5" => self.0.decrypt(rsa::Oaep::new::<Sha256>(), &data),
            k => return Err(format!("Encrypted keys of type {k} aren't supported")),
        }
        .map_err(|_| "Couldn't open an organisation's key".to_owned())?;
        let k = SymKey::from_bytes(&plain);
        plain.zeroize();
        k
    }
}

/// A new account's keys, as the official clients make them: a random user key encrypted with
/// the stretched master key, and an RSA key pair whose private half the user key encrypts.
/// (protected user key, public key as SPKI base64, encrypted private key)
#[cfg(test)]
pub fn new_account_keys(master: &Key32) -> (SymKey, String, String, String) {
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
    let user = SymKey::random();
    let protected = SymKey::stretch(master).encrypt(&user.to_bytes());
    let private = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).expect("an RSA key");
    let public = B64.encode(private.to_public_key().to_public_key_der().expect("SPKI").as_bytes());
    let der = private.to_pkcs8_der().expect("PKCS#8");
    let enc_private = user.encrypt(der.as_bytes());
    (user, protected, public, enc_private)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_and_a_wrong_key_is_caught() {
        let k = SymKey::random();
        let enc = k.encrypt_str("hunter2");
        assert!(enc.starts_with("2."));
        assert_eq!(k.decrypt_str(&enc).unwrap(), "hunter2");
        assert!(SymKey::random().decrypt_str(&enc).is_err(), "the MAC catches another key");
        let mut parts: Vec<String> = enc.split('|').map(str::to_owned).collect();
        parts[1] = B64.encode(b"0123456789abcdef");
        assert!(k.decrypt_str(&parts.join("|")).is_err(), "and a changed ciphertext");
    }

    #[test]
    fn keys_match_an_independent_implementation() {
        // Worked out with Python's hashlib and hmac: PBKDF2-SHA256 of "asdfasdf" salted with
        // the address (5,000 rounds), its one-round hash salted with the password, and the
        // HKDF-Expand of the master key with "enc" and "mac".
        let mk = master_key("asdfasdf", "User@Example.com", Kdf::Pbkdf2 { iterations: 5000 }).unwrap();
        assert_eq!(B64.encode(mk.0), "b40VZZK+2irr7aFZI0eI/1Gz73zyjbPPwHqj2+vhVOE=");
        assert_eq!(password_hash(&mk, "asdfasdf"), "M9XvI6JjzLqF+hGFFCSfjHvVMvx+T7P6ecAsxmD6ukA=");
        let stretched = SymKey::stretch(&mk);
        assert_eq!(B64.encode(stretched.enc), "7aiqU88bfjdX3IrkOMfPWi40Bu0oGhu7M0DB14lXHtU=");
        assert_eq!(B64.encode(stretched.mac.unwrap()), "Hrt8rWD7ZGzZbZevVcVElUwic9HPJ02gOiBTNEGHUb0=");
    }

    #[test]
    fn argon2_keys_are_salted_with_the_address_hash() {
        let kdf = Kdf::Argon2id { iterations: 1, memory_mib: 8, parallelism: 1 };
        let a = master_key("pw", "Me@Example.com ", kdf).unwrap();
        let b = master_key("pw", "me@example.com", kdf).unwrap();
        assert_eq!(a.0, b.0, "the address is trimmed and lowercased");
        assert_ne!(a.0, master_key("pw", "you@example.com", kdf).unwrap().0);
    }

    #[test]
    fn a_new_accounts_keys_open_again() {
        let mk = master_key("correct horse", "a@b.c", Kdf::Pbkdf2 { iterations: 1000 }).unwrap();
        let (user, protected, _public, enc_private) = new_account_keys(&mk);
        let opened = SymKey::stretch(&mk).decrypt_key(&protected).unwrap();
        assert_eq!(opened.to_bytes(), user.to_bytes());
        let private = PrivateKey::open(&enc_private, &opened).unwrap();
        // An organisation key, as the server would hand it to this account.
        use rsa::traits::PublicKeyParts;
        let org = SymKey::random();
        let public = private.0.to_public_key();
        assert!(public.size() == 256);
        let enc = rsa::Oaep::new::<sha1::Sha1>();
        let data = public.encrypt(&mut rsa::rand_core::OsRng, enc, &org.to_bytes()).unwrap();
        let back = private.decrypt_key(&format!("4.{}", B64.encode(data))).unwrap();
        assert_eq!(back.to_bytes(), org.to_bytes());
    }
}
