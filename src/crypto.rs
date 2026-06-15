pub const SLUG_LEN: usize = 12;

pub fn is_valid_slug(slug: &str) -> bool {
    slug.len() == SLUG_LEN && slug.chars().all(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_slug_passes() {
        assert!(is_valid_slug("abc123DEF456"));
    }

    #[test]
    fn short_slug_fails() {
        assert!(!is_valid_slug("abc123"));
    }

    #[test]
    fn invalid_char_slug_fails() {
        assert!(!is_valid_slug("abc123DEF45-"));
    }

    #[test]
    fn empty_slug_fails() {
        assert!(!is_valid_slug(""));
    }
}
