use rand::RngCore;

const SLUG_LEN: usize = 12;
const KEY_LEN: usize = 32;

pub fn generate_slug() -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    let mut bytes = vec![0u8; SLUG_LEN];
    rng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|b| CHARSET[(b % CHARSET.len() as u8) as usize] as char)
        .collect()
}

pub fn generate_key() -> Vec<u8> {
    let mut key = vec![0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut key);
    key
}
