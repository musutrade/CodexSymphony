//! Authentication primitives; no runtime or business lifecycle dependencies.
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

pub const COOKIE: &str = "__Host-codexsession";
pub const SESSION_SECONDS: i64 = 8 * 60 * 60;
pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn random() -> Result<String, &'static str> {
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .or(Err("random source unavailable"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn csrf(token: &str) -> String {
    digest(&format!("csrf:{token}"))
}
pub fn valid_username(value: &str) -> bool {
    (3..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
pub fn hash(password: &str) -> Result<String, &'static str> {
    if !(12..=1024).contains(&password.len()) {
        return Err("password must contain 12 to 1024 bytes");
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .or(Err("password hashing failed"))
}
pub fn verify(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}
pub fn cookie(token: &str, seconds: i64) -> String {
    format!("{COOKIE}={token}; Path=/; Max-Age={seconds}; HttpOnly; Secure; SameSite=Lax")
}
