//! Single-admin authentication with an in-memory session store.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::RngCore;

use crate::config::AdminAccount;

pub const COOKIE: &str = "tushonka_session";
/// Sessions expire after this much inactivity.
pub const IDLE: Duration = Duration::from_secs(8 * 3600);

pub struct Auth {
    admin: Mutex<Option<AdminAccount>>,
    sessions: Mutex<HashMap<String, Instant>>,
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

impl Auth {
    pub fn new(admin: Option<AdminAccount>) -> Self {
        Self { admin: Mutex::new(admin), sessions: Mutex::default(), failures: Mutex::default() }
    }

    pub fn setup_required(&self) -> bool {
        self.admin.lock().unwrap().is_none()
    }

    pub fn username(&self) -> Option<String> {
        self.admin.lock().unwrap().as_ref().map(|a| a.username.clone())
    }

    /// First-run: creates the admin account. Fails if one already exists.
    pub fn setup(&self, username: &str, password: &str) -> Result<AdminAccount> {
        let mut admin = self.admin.lock().unwrap();
        if admin.is_some() {
            return Err(anyhow!("an admin account already exists"));
        }
        let username = username.trim();
        if username.is_empty() || username.len() > 64 {
            return Err(anyhow!("username must be 1-64 characters"));
        }
        if password.chars().count() < 8 {
            return Err(anyhow!("password must be at least 8 characters"));
        }
        let account = AdminAccount { username: username.to_string(), password_hash: hash_password(password)? };
        *admin = Some(account.clone());
        Ok(account)
    }

    /// Checks credentials. Repeated failures from one client are slowed down by the caller.
    pub fn verify(&self, username: &str, password: &str) -> bool {
        let admin = self.admin.lock().unwrap().clone();
        let Some(admin) = admin else { return false };
        let Ok(parsed) = PasswordHash::new(&admin.password_hash) else { return false };
        let ok_pw = Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok();
        ok_pw && admin.username == username.trim()
    }

    /// Records a failed attempt and returns how long the caller should wait before answering.
    pub fn note_failure(&self, client: &str) -> Duration {
        let mut f = self.failures.lock().unwrap();
        let now = Instant::now();
        let entry = f.entry(client.to_string()).or_insert((0, now));
        if now.duration_since(entry.1) > Duration::from_secs(900) {
            *entry = (0, now);
        }
        entry.0 += 1;
        entry.1 = now;
        Duration::from_millis(400 * u64::from(entry.0.min(15)))
    }

    pub fn clear_failures(&self, client: &str) {
        self.failures.lock().unwrap().remove(client);
    }

    pub fn create_session(&self) -> String {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let mut s = self.sessions.lock().unwrap();
        s.retain(|_, last| last.elapsed() < IDLE);
        s.insert(token.clone(), Instant::now());
        token
    }

    /// Validates and refreshes a session.
    pub fn touch(&self, token: &str) -> bool {
        let mut s = self.sessions.lock().unwrap();
        match s.get_mut(token) {
            Some(last) if last.elapsed() < IDLE => {
                *last = Instant::now();
                true
            }
            Some(_) => {
                s.remove(token);
                false
            }
            None => false,
        }
    }

    pub fn end_session(&self, token: &str) {
        self.sessions.lock().unwrap().remove(token);
    }
}

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow!("hashing failed: {e}"))
}

/// Extracts our session token from a `Cookie` header.
pub fn token_from_cookie(header: &str) -> Option<&str> {
    header.split(';').map(str::trim).find_map(|kv| kv.strip_prefix(COOKIE)?.strip_prefix('='))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_login_session() {
        let auth = Auth::new(None);
        assert!(auth.setup_required());
        assert!(auth.setup("admin", "short").is_err());
        auth.setup("admin", "correct horse").unwrap();
        assert!(auth.setup("again", "correct horse").is_err());
        assert!(auth.verify("admin", "correct horse"));
        assert!(!auth.verify("admin", "wrong password"));
        assert!(!auth.verify("other", "correct horse"));
        let t = auth.create_session();
        assert!(auth.touch(&t));
        auth.end_session(&t);
        assert!(!auth.touch(&t));
    }

    #[test]
    fn cookie_parsing() {
        assert_eq!(token_from_cookie("a=1; tushonka_session=abc; b=2"), Some("abc"));
        assert_eq!(token_from_cookie("tushonka_session_x=abc"), None);
        assert_eq!(token_from_cookie("a=1"), None);
    }
}
