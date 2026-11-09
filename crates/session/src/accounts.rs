use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use executor::{Executor, Task};
use subsonic::Credentials;

use crate::{Readings, Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInField {
    Server,
    Username,
    Password,
}

/// What is missing or wrong in what was typed, found before asking any server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid {
    field: SignInField,
    message: String,
}

impl Invalid {
    pub fn field(&self) -> SignInField {
        self.field
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[non_exhaustive]
pub struct SignedIn {
    pub session: Session,
    pub profile: config::Profile,
}

/// The profiles and passwords the user signs in with, and signing in with them.
#[derive(Clone)]
pub struct Accounts(Arc<Inner>);

struct Inner {
    executor: Executor,
    net: net::Client,
    /// `None` without a home directory: the form then starts empty.
    store: Option<config::Store>,
    /// `None` where the system has no credential store; signing in then
    /// always asks for the password.
    keychain: Option<config::Keychain>,
    readings: Option<Readings>,
    last_profile: Mutex<Option<config::Profile>>,
}

impl Accounts {
    /// Every session it signs in to reads kanji with `readings`.
    pub fn new(
        executor: Executor,
        net: net::Client,
        store: Result<config::Store, config::Error>,
        keychain: Option<config::Keychain>,
        readings: Option<Readings>,
    ) -> Self {
        let store = store.ok();
        let last_profile = store
            .as_ref()
            .and_then(|store| store.last_profile().ok().flatten());
        Self(Arc::new(Inner {
            executor,
            net,
            store,
            keychain,
            readings,
            last_profile: Mutex::new(last_profile),
        }))
    }

    /// The profile last signed in with.
    pub fn last_profile(&self) -> Option<config::Profile> {
        lock(&self.0.last_profile).clone()
    }

    /// The password written in the settings file for `profile`, or else the one in the
    /// keychain, which may ask the user first. `None` when the keychain fails.
    pub fn saved_password(&self, profile: config::Profile) -> Task<Option<String>> {
        let store = self.0.store.clone();
        let keychain = self.0.keychain.clone();
        self.0.executor.spawn_blocking(move || {
            password_in_file(store.as_ref(), &profile)
                .or_else(|| keychain?.password(&profile).ok().flatten())
        })
    }

    /// Checks what was typed before anything else. Signed in, the profile and the password are
    /// remembered for next time; the error says what went wrong for the user.
    pub fn sign_in(
        &self,
        address: &str,
        username: &str,
        password: &str,
    ) -> Result<Task<Result<SignedIn, String>>, Invalid> {
        let address = address.trim().to_owned();
        let username = username.trim().to_owned();
        let password = password.to_owned();
        let invalid = |field, message: &str| Invalid {
            field,
            message: message.to_owned(),
        };
        if address.is_empty() {
            return Err(invalid(SignInField::Server, "Enter the server address."));
        }
        if username.is_empty() {
            return Err(invalid(SignInField::Username, "Enter your username."));
        }
        if password.is_empty() {
            return Err(invalid(SignInField::Password, "Enter your password."));
        }
        let credentials = Credentials::password(username.clone(), password.clone());
        let connecting = self.connect(&address, credentials)?;
        let this = self.clone();
        Ok(self.0.executor.spawn(async move {
            let session = connecting.await?;
            let profile = config::Profile::new(address, username);
            let store = this.0.store.clone();
            this.0
                .executor
                .spawn_blocking({
                    let profile = profile.clone();
                    // A failure only means the user types more next time
                    move || store.map(|store| store.save_profile(&profile).ok())
                })
                .await;
            this.remember_password(&profile, password);
            *lock(&this.0.last_profile) = Some(profile.clone());
            Ok(SignedIn { session, profile })
        }))
    }

    /// Opens a session on the server at `address` with `credentials`, remembering nothing, as a
    /// command line does. The error says what went wrong for the user.
    pub fn connect(
        &self,
        address: &str,
        credentials: Credentials,
    ) -> Result<Task<Result<Session, String>>, Invalid> {
        let username = match &credentials {
            Credentials::Password { username, .. } => Some(username.clone()),
            Credentials::ApiKey(_) => None,
        };
        let server = client::Server::new(address, credentials).map_err(|error| Invalid {
            field: SignInField::Server,
            message: format!("{error}."),
        })?;
        let client = client::Client::new(self.0.net.clone(), server);
        let this = self.clone();
        Ok(self.0.executor.spawn(async move {
            let info = client.ping().await.map_err(|error| describe(&error))?;
            let server = client.server().base_url().clone();
            let (catalog, scrobbles) = this
                .0
                .executor
                .spawn_blocking({
                    let username = username.clone();
                    // Without its cached library the workspace loads everything from the server,
                    // and without somewhere to keep plays it sends them right away, so failing to
                    // open either is not shown.
                    move || {
                        let scrobbles = username
                            .and_then(|username| catalog::Scrobbles::user(&server, &username).ok());
                        (catalog::Catalog::open_server(&server).ok(), scrobbles)
                    }
                })
                .await;
            let mut session = Session::new(
                client,
                username.unwrap_or_default(),
                info,
                catalog,
                scrobbles,
            );
            if let Some(readings) = &this.0.readings {
                session = session.with_readings(readings.clone());
            }
            Ok(session)
        }))
    }

    /// Signing in with `profile` then asks for the password, unless the settings file has it.
    pub fn forget_password(&self, profile: config::Profile) {
        let Some(keychain) = self.0.keychain.clone() else {
            return;
        };
        self.0
            .executor
            .spawn_blocking(move || keychain.delete_password(&profile).ok())
            .detach();
    }

    /// A failure only means the user types more next time, so it is not shown. The keychain may
    /// ask the user, so nothing waits for it.
    fn remember_password(&self, profile: &config::Profile, password: String) {
        let Some(keychain) = self.0.keychain.clone() else {
            return;
        };
        let store = self.0.store.clone();
        let profile = profile.clone();
        self.0
            .executor
            .spawn_blocking(move || {
                if password_in_file(store.as_ref(), &profile).is_none() {
                    keychain.set_password(&profile, &password).ok();
                }
            })
            .detach();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn password_in_file(store: Option<&config::Store>, profile: &config::Profile) -> Option<String> {
    store?.password(profile).ok().flatten()
}

fn describe(error: &client::Error) -> String {
    match error {
        client::Error::Protocol(subsonic::Error::Server { code, .. })
            if code.is_authentication() =>
        {
            "Wrong username or password.".to_owned()
        }
        client::Error::Protocol(subsonic::Error::Server { message, .. }) => {
            format!("The server refused to sign in: {message}")
        }
        client::Error::Protocol(subsonic::Error::InvalidResponse(_))
        | client::Error::HttpStatus(_)
        | client::Error::NotAudio(_) => {
            "No Subsonic server answered at this address. Check the address.".to_owned()
        }
        client::Error::Network(_) => {
            "Couldn't reach the server. Check the address and your connection.".to_owned()
        }
        client::Error::InvalidServerUrl(_) | client::Error::ServerUrlHasExtras(_) => {
            format!("{error}.")
        }
    }
}

#[cfg(test)]
mod tests;
