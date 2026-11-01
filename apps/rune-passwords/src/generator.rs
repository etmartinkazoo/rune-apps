//! New passwords, as Apple's strong passwords are made: letters and digits in hyphenated
//! groups of six (`hufpyb-6xamqo-tisgEk`), one capital and one digit at least, easy to read
//! and to type on a phone, and some 80 bits of randomness: far past what guessing can reach.

const LOWER: &[u8] = b"abcdefghjkmnpqrstuvwxyz";
const UPPER: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const DIGITS: &[u8] = b"23456789";

/// A random index below `n`, without bias.
fn below(n: usize) -> usize {
    let zone = u32::MAX - u32::MAX % n as u32;
    loop {
        let mut b = [0u8; 4];
        getrandom::fill(&mut b).expect("the system's random source");
        let v = u32::from_le_bytes(b);
        if v < zone {
            return (v % n as u32) as usize;
        }
    }
}

fn pick(set: &[u8]) -> char {
    set[below(set.len())] as char
}

/// Three groups of six, hyphenated.
pub fn generate() -> String {
    let mut chars: Vec<char> = (0..18).map(|_| pick(LOWER)).collect();
    // One capital and one digit, at places of their own.
    let a = below(18);
    let mut b = below(18);
    while b == a {
        b = below(18);
    }
    chars[a] = pick(UPPER);
    chars[b] = pick(DIGITS);
    chars.chunks(6).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_has_what_it_promises() {
        for _ in 0..50 {
            let g = generate();
            assert_eq!(g.len(), 20);
            assert_eq!(g.matches('-').count(), 2);
            assert!(g.chars().any(|c| c.is_ascii_uppercase()) && g.chars().any(|c| c.is_ascii_digit()));
        }
        assert_ne!(generate(), generate());
    }
}
