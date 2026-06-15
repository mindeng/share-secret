use rand::Rng;

pub const SLUG_LEN: usize = 12;

const SLUG_CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

pub fn generate_slug() -> String {
    let mut rng = rand::thread_rng();
    (0..SLUG_LEN)
        .map(|_| SLUG_CHARSET[rng.gen_range(0..SLUG_CHARSET.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_slug_has_correct_length_and_charset() {
        let slug = generate_slug();
        assert_eq!(slug.len(), SLUG_LEN);
        assert!(slug.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
