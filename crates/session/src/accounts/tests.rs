use executor::testing::TestExecutor;

use super::*;

const SERVER: &str = "https://music.example.com";

fn alice() -> config::Profile {
    config::Profile::new(SERVER, "alice")
}

/// With the settings file in `dir` and a keychain of its own.
fn accounts(dir: &tempfile::TempDir, keychain: &config::Keychain, cx: &TestExecutor) -> Accounts {
    let executor = cx.executor();
    let net = net::Client::new("hiphile-test").unwrap();
    let store = Ok(config::Store::at(dir.path()));
    Accounts::new(executor, net, store, Some(keychain.clone()))
}

fn invalid(
    accounts: &Accounts,
    address: &str,
    username: &str,
    password: &str,
) -> Option<(SignInField, String)> {
    let invalid = accounts.sign_in(address, username, password).err()?;
    Some((invalid.field(), invalid.message().to_owned()))
}

#[test]
fn what_is_missing_is_asked_for_in_order() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let accounts = accounts(&dir, &config::Keychain::in_memory(), cx);

    let missing = |address, username, password| {
        invalid(&accounts, address, username, password).map(|(field, _)| field)
    };
    assert_eq!(missing(" ", "alice", "sesame"), Some(SignInField::Server));
    assert_eq!(missing(SERVER, " ", "sesame"), Some(SignInField::Username));
    assert_eq!(missing(SERVER, "alice", ""), Some(SignInField::Password));
    assert_eq!(
        invalid(&accounts, "", "", "").unwrap().1,
        "Enter the server address."
    );
}

#[test]
fn an_unusable_address_is_told_as_a_sentence() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let accounts = accounts(&dir, &config::Keychain::in_memory(), cx);

    let (field, message) = invalid(&accounts, "ftp://music.example.com", "alice", "x").unwrap();
    assert_eq!(field, SignInField::Server);
    assert!(message.ends_with('.'));
}

#[test]
fn connecting_to_an_unusable_address_is_told_as_a_sentence() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let accounts = accounts(&dir, &config::Keychain::in_memory(), cx);

    let invalid = accounts
        .connect("ftp://music.example.com", Credentials::api_key("k"))
        .err()
        .unwrap();

    assert_eq!(invalid.field(), SignInField::Server);
    assert!(invalid.message().ends_with('.'));
}

#[test]
fn the_last_profile_is_read_when_made() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    config::Store::at(dir.path())
        .save_profile(&alice())
        .unwrap();

    let accounts = accounts(&dir, &config::Keychain::in_memory(), cx);

    assert_eq!(accounts.last_profile(), Some(alice()));
}

#[test]
fn a_password_in_the_file_comes_before_the_keychain() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let keychain = config::Keychain::in_memory();
    keychain.set_password(&alice(), "from-keychain").unwrap();
    let accounts = accounts(&dir, &keychain, cx);

    let mut saved = accounts.saved_password(alice());
    cx.run_until_parked();
    assert_eq!(
        futures::FutureExt::now_or_never(&mut saved).flatten(),
        Some("from-keychain".to_owned())
    );

    let file = format!("server = \"{SERVER}\"\nusername = \"alice\"\npassword = \"from-file\"\n");
    std::fs::write(dir.path().join("config.toml"), file).unwrap();
    let mut saved = accounts.saved_password(alice());
    cx.run_until_parked();
    assert_eq!(
        futures::FutureExt::now_or_never(&mut saved).flatten(),
        Some("from-file".to_owned())
    );
}

#[test]
fn a_password_kept_under_the_address_as_typed_is_still_found() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let typed = format!("{SERVER}/");
    let file = format!("server = \"{typed}\"\nusername = \"alice\"\n");
    std::fs::write(dir.path().join("config.toml"), file).unwrap();
    let keychain = config::Keychain::in_memory();
    keychain
        .set_password_as_written("alice", &typed, "sesame")
        .unwrap();
    let accounts = accounts(&dir, &keychain, cx);

    let mut saved = accounts.saved_password(alice());
    cx.run_until_parked();
    assert_eq!(
        futures::FutureExt::now_or_never(&mut saved).flatten(),
        Some("sesame".to_owned())
    );
    assert_eq!(
        keychain.password(&alice()).unwrap().as_deref(),
        Some("sesame")
    );
    assert_eq!(keychain.adopt_password(&alice(), &typed).unwrap(), None);
}

#[test]
fn forgetting_a_profile_removes_the_password_kept_under_the_address_as_typed() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let typed = format!("{SERVER}/");
    let file = format!("server = \"{typed}\"\nusername = \"alice\"\n");
    std::fs::write(dir.path().join("config.toml"), file).unwrap();
    let keychain = config::Keychain::in_memory();
    keychain
        .set_password_as_written("alice", &typed, "sesame")
        .unwrap();
    let accounts = accounts(&dir, &keychain, cx);

    accounts.forget(alice());
    cx.run_until_parked();

    assert_eq!(keychain.adopt_password(&alice(), &typed).unwrap(), None);
    assert_eq!(keychain.password(&alice()).unwrap(), None);
}

#[test]
fn the_profiles_are_read_when_made() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let bob = config::Profile::new(SERVER, "bob");
    let store = config::Store::at(dir.path());
    store.save_profile(&alice()).unwrap();
    store.save_profile(&bob).unwrap();

    let accounts = accounts(&dir, &config::Keychain::in_memory(), cx);

    assert_eq!(accounts.profiles(), [alice(), bob]);
}

#[test]
fn a_forgotten_profile_is_gone_with_its_password() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let bob = config::Profile::new(SERVER, "bob");
    let store = config::Store::at(dir.path());
    store.save_profile(&bob).unwrap();
    store.save_profile(&alice()).unwrap();
    let keychain = config::Keychain::in_memory();
    keychain.set_password(&alice(), "sesame").unwrap();
    let accounts = accounts(&dir, &keychain, cx);

    accounts.forget(alice());
    assert_eq!(accounts.profiles(), std::slice::from_ref(&bob));
    cx.run_until_parked();

    assert_eq!(keychain.password(&alice()).unwrap(), None);
    assert_eq!(store.profiles().unwrap(), [bob]);
    assert_eq!(accounts.last_profile(), Some(alice()));
}

#[test]
fn a_refused_sign_in_says_what_to_check() {
    let wrong = subsonic::Error::Server {
        code: subsonic::ServerErrorCode::new(40),
        message: "Wrong username or password".to_owned(),
    };
    assert_eq!(
        describe(&client::Error::Protocol(wrong)),
        "Wrong username or password."
    );
    assert_eq!(
        describe(&client::Error::HttpStatus(404)),
        "No Subsonic server answered at this address. Check the address."
    );
}
