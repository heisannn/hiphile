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

    /// The password in the entry named after `server`, which is `profile`'s address written
    /// another way, moved to `profile`'s own entry. `None` when there is no such entry.
    pub fn adopt_password(&self, profile: &Profile, server: &str) -> Result<Option<String>, Error> {
        let written = self.entry_named(profile.username(), server)?;
        let password = match written.get_password() {
            Ok(password) => password,
            Err(keyring_core::Error::NoEntry) => return Ok(None),
            Err(error) => return Err(Error::Keychain(error)),
        };
        self.set_password(profile, &password)?;
        // Left where it is if it cannot be removed, as the password is already in its new place
        written.delete_credential().ok();
        Ok(Some(password))
    }

    /// Removes the entry named after `server`, which is `profile`'s address written another
    /// way. Succeeds if there was none.
    pub fn delete_password_as_written(&self, profile: &Profile, server: &str) -> Result<(), Error> {
        match self
            .entry_named(profile.username(), server)?
            .delete_credential()
        {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(Error::Keychain(error)),
        }
    }

    fn entry(&self, profile: &Profile) -> Result<Entry, Error> {
        self.entry_named(profile.username(), profile.server())
    }

    /// One entry per server and user, named `user@server` so it reads well
    /// in the system's password manager.
    fn entry_named(&self, username: &str, server: &str) -> Result<Entry, Error> {
        let account = format!("{username}@{server}");
        self.store
            .build(SERVICE, &account, None)
            .map_err(Error::Keychain)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Keychain {
    /// Keeps `password` in an entry named after `server` as typed, as versions up to 0.5.0 did.
    pub fn set_password_as_written(
        &self,
        username: &str,
        server: &str,
        password: &str,
    ) -> Result<(), Error> {
        self.entry_named(username, server)?
            .set_password(password)
            .map_err(Error::Keychain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_kept_under_the_address_as_typed_moves_to_the_profile() {
        let keychain = Keychain::in_memory();
        let alice = Profile::new("https://music.example.com", "alice");
        let typed = "https://Music.Example.com/";
        keychain
            .set_password_as_written("alice", typed, "sesame")
            .unwrap();
        assert_eq!(keychain.password(&alice).unwrap(), None);

        assert_eq!(
            keychain.adopt_password(&alice, typed).unwrap().as_deref(),
            Some("sesame")
        );
        assert_eq!(
            keychain.password(&alice).unwrap().as_deref(),
            Some("sesame")
        );
        assert_eq!(keychain.adopt_password(&alice, typed).unwrap(), None);
    }

    #[test]
    fn a_password_kept_under_the_address_as_typed_is_removed_by_that_name() {
        let keychain = Keychain::in_memory();
        let alice = Profile::new("https://music.example.com", "alice");
        let typed = "https://music.example.com/";
        keychain
            .set_password_as_written("alice", typed, "sesame")
            .unwrap();

        keychain.delete_password_as_written(&alice, typed).unwrap();
        assert_eq!(keychain.adopt_password(&alice, typed).unwrap(), None);
        keychain.delete_password_as_written(&alice, typed).unwrap();
    }

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
