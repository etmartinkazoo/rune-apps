//! Phone numbers as people and phones write them: "+1 (555) 010-0123", "555-0100", "05550100".
//! Two refer to the same line when their last nine digits match (enough to tell lines apart,
//! few enough to ignore a country code or trunk prefix written one way and not the other).

/// The digits that identify a line, for matching.
pub fn key(n: &str) -> String {
    let digits: String = n.chars().filter(char::is_ascii_digit).collect();
    digits[digits.len().saturating_sub(9)..].to_owned()
}

/// A number from how a phone addresses a message: RCS chats can come as `tel:+15550100123` or
/// `sip:+15550100123@ims.mnc260.mcc310.3gppnetwork.org`, whose domain's digits aren't the number.
pub fn address(a: &str) -> String {
    let a = a.trim();
    let lower = a.to_ascii_lowercase();
    let Some(rest) = ["tel:", "sip:", "sips:"].iter().find_map(|s| lower.starts_with(s).then(|| &a[s.len()..])) else { return a.to_owned() };
    rest.split(['@', ';']).next().unwrap_or("").to_owned()
}

#[cfg(test)]
pub fn same(a: &str, b: &str) -> bool {
    let (a, b) = (key(a), key(b));
    !a.is_empty() && a == b
}

/// What `Dial` takes: digits, `+`, `*`, `#`, `,` only (PipeWire checks
/// `[0-9+*#,ABCD]{1,80}`).
pub fn dialable(n: &str) -> Option<String> {
    let s: String = n.chars().filter(|c| c.is_ascii_digit() || matches!(c, '+' | '*' | '#' | ',')).collect();
    (!s.is_empty() && s.len() <= 80).then_some(s)
}

/// A number laid out for reading: North American ones as (555) 010-0123, others as written.
pub fn pretty(n: &str) -> String {
    let digits: String = n.chars().filter(char::is_ascii_digit).collect();
    match (n.trim().starts_with('+'), digits.len()) {
        (false, 10) => format!("({}) {}-{}", &digits[..3], &digits[3..6], &digits[6..]),
        (true, 11) if digits.starts_with('1') => format!("+1 ({}) {}-{}", &digits[1..4], &digits[4..7], &digits[7..]),
        _ => n.trim().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_dials_and_reads() {
        assert!(same("+1 (555) 010-0123", "5550100123"));
        assert!(!same("", ""));
        assert_eq!(dialable("+1 (555) 010-0123").as_deref(), Some("+15550100123"));
        assert_eq!(dialable("call me"), None);
        assert_eq!(pretty("5550100123"), "(555) 010-0123");
        assert_eq!(pretty("+15550100123"), "+1 (555) 010-0123");
        assert_eq!(pretty("+44 7700 900123"), "+44 7700 900123");
        assert_eq!(address("sip:+15550100123@ims.mnc260.mcc310.3gppnetwork.org"), "+15550100123");
        assert_eq!(address("tel:+15550100123;phone-context=x"), "+15550100123");
        assert_eq!(address("+1 555 010 0123"), "+1 555 010 0123");
    }
}
