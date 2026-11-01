//! How good a password is: guessable (zxcvbn's judgement, with the item's name and username
//! counted against it) or used by more than one item.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    Weak,
    Fair,
    Strong,
}

/// zxcvbn's score of 0 to 4: 0-2 weak, 3 fair, 4 strong. The item's own words make it weaker.
pub fn strength(password: &str, context: &[&str]) -> Strength {
    if password.is_empty() {
        return Strength::Weak;
    }
    let words: Vec<&str> = context.iter().flat_map(|c| c.split(|ch: char| !ch.is_alphanumeric())).filter(|w| w.len() > 2).collect();
    match zxcvbn::zxcvbn(password, &words).score() {
        zxcvbn::Score::Four => Strength::Strong,
        zxcvbn::Score::Three => Strength::Fair,
        _ => Strength::Weak,
    }
}

/// For each password used by more than one item, how many use it.
pub fn reused<'a>(passwords: impl IntoIterator<Item = &'a str>) -> HashMap<&'a str, usize> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for p in passwords.into_iter().filter(|p| !p.is_empty()) {
        *counts.entry(p).or_default() += 1;
    }
    counts.retain(|_, n| *n > 1);
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judges_and_counts() {
        assert_eq!(strength("password1", &[]), Strength::Weak);
        assert_eq!(strength("etsy2024!", &["Etsy"]), Strength::Weak, "the site's own name doesn't count");
        assert_eq!(strength("vH7#qL2m!xR9@wZ4", &[]), Strength::Strong);
        let r = reused(["a1", "b2", "a1", ""]);
        assert_eq!(r.get("a1"), Some(&2));
        assert!(!r.contains_key("b2"));
    }
}
