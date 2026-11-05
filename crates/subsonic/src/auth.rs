use md5::{Digest, Md5};

#[derive(Clone)]
pub enum Credentials {
    /// Token authentication: `t = md5(password + salt)` with a fresh salt per request.
    Password { username: String, password: String },
    /// OpenSubsonic `apiKeyAuthentication` extension.
    ApiKey(String),
}

impl Credentials {
    pub fn password(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self::Password {
            username: username.into(),
            password: password.into(),
        }
    }

    pub fn api_key(key: impl Into<String>) -> Self {
        Self::ApiKey(key.into())
    }

    pub(crate) fn query_pairs(&self) -> Vec<(&'static str, String)> {
        self.query_pairs_with_salt(&hex::encode(rand::random::<[u8; 8]>()))
    }

    fn query_pairs_with_salt(&self, salt: &str) -> Vec<(&'static str, String)> {
        match self {
            Self::Password { username, password } => vec![
                ("u", username.clone()),
                ("t", token(password, salt)),
                ("s", salt.to_owned()),
            ],
            Self::ApiKey(key) => vec![("apiKey", key.clone())],
        }
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password { username, .. } => f
                .debug_struct("Password")
                .field("username", username)
                .finish_non_exhaustive(),
            Self::ApiKey(_) => f.write_str("ApiKey(..)"),
        }
    }
}

fn token(password: &str, salt: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(password.as_bytes());
    hasher.update(salt.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_matches_subsonic_reference_vector() {
        // From the Subsonic API documentation
        assert_eq!(
            token("sesame", "c19b2d"),
            "26719a1196d2a940705a59634eb18eab"
        );
    }

    #[test]
    fn password_credentials_use_token_and_salt() {
        let pairs = Credentials::password("admin", "sesame").query_pairs_with_salt("c19b2d");
        assert_eq!(
            pairs,
            vec![
                ("u", "admin".to_owned()),
                ("t", "26719a1196d2a940705a59634eb18eab".to_owned()),
                ("s", "c19b2d".to_owned()),
            ]
        );
    }

    #[test]
    fn debug_never_prints_secrets() {
        let debug = format!(
            "{:?} {:?}",
            Credentials::password("admin", "sesame"),
            Credentials::api_key("secret-key")
        );
        assert!(!debug.contains("sesame"));
        assert!(!debug.contains("secret-key"));
    }
}
