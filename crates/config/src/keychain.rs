use std::sync::Arc;

use keyring_core::{CredentialStore, Entry};

use crate::{Error, Profile};

const SERVICE: &str = "hiphile";

/// Every call blocks, and the system may ask the user to allow access, so
/// call it off the UI thread.
#[derive(Clone)]
pub struct Keychain {
    store: Arc<CredentialStore>,
}

impl Keychain {
    /// Fails where there is none, such as a Linux system without a Secret Service.
    pub fn system() -> Result<Self, Error> {
        #[cfg(target_os = "macos")]
        let store = apple_native_keyring_store::keychain::Store::new();
        #[cfg(target_os = "windows")]
        let store = windows_native_keyring_store::Store::new();
        #[cfg(all(unix, not(target_os = "macos")))]
        let store = zbus_secret_service_keyring_store::Store::new();
        Ok(Self {
            store: store.map_err(Error::Keychain)?,
        })
    }

    pub fn in_memory() -> Self {
        Self {
            store: keyring_core::mock::Store::new().expect("the mock store always builds"),
        }
    }

    pub fn password(&self, profile: &Profile) -> Result<Option<String>, Error> {
        match self.entry(profile)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(Error::Keychain(error)),
        }
    }

    pub fn set_password(&self, profile: &Profile, password: &str) -> Result<(), Error> {
        self.entry(profile)?
            .set_password(password)
            .map_err(Error::Keychain)
    }

    /// Succeeds if there was none.
    pub fn delete_password(&self, profile: &Profile) -> Result<(), Error> {
        match self.entry(profile)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(Error::Keychain(error)),
        }
    }

    /// One entry per server and user, named `user@server` so it reads well
    /// in the system's password manager.
    fn entry(&self, profile: &Profile) -> Result<Entry, Error> {
        let account = format!("{}@{}", profile.username(), profile.server());
        self.store
            .build(SERVICE, &account, None)
            .map_err(Error::Keychain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_kept_per_profile() {
        let keychain = Keychain::in_memory();
        let alice = Profile::new("https://music.example.com", "alice");
        let bob = Profile::new("https://music.example.com", "bob");
        assert_eq!(keychain.password(&alice).unwrap(), None);

        keychain.set_password(&alice, "sesame").unwrap();
        assert_eq!(
            keychain.password(&alice).unwrap().as_deref(),
            Some("sesame")
        );
        assert_eq!(keychain.password(&bob).unwrap(), None);

        keychain.delete_password(&alice).unwrap();
        assert_eq!(keychain.password(&alice).unwrap(), None);
        keychain.delete_password(&alice).unwrap();
    }
}
