//! Verification codes (TOTP, RFC 6238) from what an item stores: a bare base32 secret, an
//! `otpauth://totp/…` URI with its digits, period and algorithm, or a `steam://` secret.

use hmac::{Hmac, Mac};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Totp {
    secret: Vec<u8>,
    digits: u32,
    pub period: u64,
    algorithm: Algorithm,
    steam: bool,
}

const STEAM: &[u8] = b"23456789BCDFGHJKMNPQRTVWXY";

impl Totp {
    pub fn parse(stored: &str) -> Option<Totp> {
        let stored = stored.trim();
        if let Some(secret) = stored.strip_prefix("steam://") {
            return Some(Totp { secret: base32(secret)?, digits: 5, period: 30, algorithm: Algorithm::Sha1, steam: true });
        }
        if stored.to_ascii_lowercase().starts_with("otpauth://") {
            let url = url::Url::parse(stored).ok()?;
            let q = |k: &str| url.query_pairs().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.into_owned());
            let algorithm = match q("algorithm").unwrap_or_default().to_ascii_uppercase().as_str() {
                "SHA256" => Algorithm::Sha256,
                "SHA512" => Algorithm::Sha512,
                _ => Algorithm::Sha1,
            };
            return Some(Totp {
                secret: base32(&q("secret")?)?,
                digits: q("digits").and_then(|d| d.parse().ok()).filter(|d| (1..=10).contains(d)).unwrap_or(6),
                period: q("period").and_then(|p| p.parse().ok()).filter(|p| *p > 0).unwrap_or(30),
                algorithm,
                steam: false,
            });
        }
        Some(Totp { secret: base32(stored)?, digits: 6, period: 30, algorithm: Algorithm::Sha1, steam: false })
    }

    /// The code at `unix` seconds.
    pub fn code(&self, unix: u64) -> String {
        let counter = (unix / self.period).to_be_bytes();
        let hash = match self.algorithm {
            Algorithm::Sha1 => mac::<Hmac<sha1::Sha1>>(&self.secret, &counter),
            Algorithm::Sha256 => mac::<Hmac<sha2::Sha256>>(&self.secret, &counter),
            Algorithm::Sha512 => mac::<Hmac<sha2::Sha512>>(&self.secret, &counter),
        };
        let offset = (hash[hash.len() - 1] & 0x0f) as usize;
        let n = u32::from_be_bytes([hash[offset] & 0x7f, hash[offset + 1], hash[offset + 2], hash[offset + 3]]);
        if self.steam {
            let mut n = n as usize;
            return (0..5)
                .map(|_| {
                    let c = STEAM[n % STEAM.len()] as char;
                    n /= STEAM.len();
                    c
                })
                .collect();
        }
        let code = u64::from(n) % 10u64.pow(self.digits);
        format!("{code:0width$}", width = self.digits as usize)
    }

    /// Seconds the code at `unix` has left.
    pub fn remaining(&self, unix: u64) -> u64 {
        self.period - unix % self.period
    }
}

fn mac<M: Mac + hmac::digest::KeyInit>(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut m = <M as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(msg);
    m.finalize().into_bytes().to_vec()
}

/// RFC 4648 base32, forgiving of case, spaces and padding.
fn base32(s: &str) -> Option<Vec<u8>> {
    let mut bits = 0u64;
    let mut n = 0;
    let mut out = Vec::new();
    for ch in s.chars().filter(|c| !c.is_whitespace() && *c != '=' && *c != '-') {
        let v = match ch.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u64 - 'A' as u64,
            c @ '2'..='7' => c as u64 - '2' as u64 + 26,
            _ => return None,
        };
        bits = (bits << 5) | v;
        n += 5;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_6238_vectors() {
        // The RFC's SHA-1 secret, "12345678901234567890", in base32.
        let t = Totp::parse("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=8").unwrap();
        assert_eq!(t.code(59), "94287082");
        assert_eq!(t.code(1111111109), "07081804");
        assert_eq!(t.code(20000000000), "65353130");
        let sha256 = Totp::parse(
            "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA&digits=8&algorithm=SHA256",
        )
        .unwrap();
        assert_eq!(sha256.code(59), "46119246");
    }

    #[test]
    fn bare_secrets_and_steam() {
        let t = Totp::parse("gezd gnbv gy3t qojq gezd gnbv gy3t qojq").unwrap();
        assert_eq!(t.code(59), "287082", "six digits by default");
        assert_eq!(t.remaining(59), 1);
        assert_eq!(Totp::parse("steam://GEZDGNBVGY3TQOJQ").unwrap().code(59).len(), 5);
        assert_eq!(Totp::parse("not base32!"), None);
    }
}
