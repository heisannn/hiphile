use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use sha2::{Digest as _, Sha256};
use tempfile::TempDir;

use super::*;

const PLATFORM: &str = "macos-arm64";

fn make_bundle(dir: &Path, version: &str) -> PathBuf {
    let app = dir.join("hiphile.app");
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS")).unwrap();
    fs::create_dir_all(contents.join("Resources")).unwrap();
    fs::write(
        contents.join("Info.plist"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
         <key>CFBundleIdentifier</key><string>dev.heisannn.hiphile</string>\
         <key>CFBundleExecutable</key><string>hiphile</string></dict></plist>",
    )
    .unwrap();
    fs::write(contents.join("Resources/marker"), version).unwrap();
    let executable = contents.join("MacOS/hiphile");
    // A real executable, as a script's signature would live in extended attributes
    fs::copy("/usr/bin/true", &executable).unwrap();
    sign(&app);
    app
}

fn sign(app: &Path) {
    run(Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(app));
}

fn run(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// As `script/package-macos` makes it.
fn zip_of(app: &Path) -> Vec<u8> {
    let zip = app.with_extension("zip");
    run(Command::new("ditto")
        .args(["-c", "-k", "--norsrc", "--noextattr", "--keepParent"])
        .arg(app)
        .arg(&zip));
    fs::read(zip).unwrap()
}

/// Serves one release after a release whose tag is no version, as the repository has.
fn serve(version: &str, assets: Vec<(String, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let listed: Vec<String> = assets
        .iter()
        .map(|(name, _)| {
            format!(
                r#"{{"name":"{name}","browser_download_url":"{base}/download/{name}","url":"{base}/download/{name}"}}"#
            )
        })
        .collect();
    let releases = format!(
        r#"[{{"tag_name":"readings-1","name":"readings-1","created_at":"2026-01-02T00:00:00Z","published_at":"2026-01-02T00:00:00Z","assets":[]}},{{"tag_name":"v{version}","name":"v{version}","created_at":"2026-01-01T00:00:00Z","published_at":"2026-01-01T00:00:00Z","assets":[{}]}}]"#,
        listed.join(",")
    );
    let served = Arc::new((releases, assets));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let served = served.clone();
            std::thread::spawn(move || answer(stream, &served.0, &served.1));
        }
    });
    base
}

fn answer(mut stream: TcpStream, releases: &str, assets: &[(String, Vec<u8>)]) {
    let mut request = [0; 8192];
    let read = stream.read(&mut request).unwrap_or(0);
    let request = String::from_utf8_lossy(&request[..read]);
    let path = request.split_whitespace().nth(1).unwrap_or_default();
    let body: &[u8] = match path.strip_prefix("/download/") {
        Some(name) => assets
            .iter()
            .find(|(asset, _)| asset == name)
            .map_or(&[][..], |(_, bytes)| bytes),
        None if path.contains("page=2") => b"[]",
        None => releases.as_bytes(),
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).ok();
    stream.write_all(body).ok();
}

fn sums_of(name: &str, zip: &[u8]) -> Vec<u8> {
    format!("{}  {name}\n", hex::encode(Sha256::digest(zip))).into_bytes()
}

/// A release of `zip` listed after a disk image, as the real ones are.
struct Fixture {
    _dir: TempDir,
    installed: PathBuf,
    installation: Installation,
}

fn fixture(version: &str, zip: Vec<u8>, sums: Option<Vec<u8>>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let applications = dir.path().join("Applications");
    fs::create_dir(&applications).unwrap();
    let installed = make_bundle(&applications, "old");
    fs::write(installed.join("Contents/Resources/stale"), "old").unwrap();
    sign(&installed);

    let name = format!("hiphile-{version}-{PLATFORM}.zip");
    let sums = sums.unwrap_or_else(|| sums_of(&name, &zip));
    let base = serve(
        version,
        vec![
            (
                format!("hiphile-{version}-{PLATFORM}.dmg"),
                b"not a zip".to_vec(),
            ),
            (name, zip),
            ("SHA256SUMS".to_owned(), sums),
        ],
    );
    let installation = Installation {
        api_base_url: Some(base),
        platform: PLATFORM.to_owned(),
        current_version: "1.0.0".to_owned(),
        bundle: Some(installed.clone()),
    };
    Fixture {
        _dir: dir,
        installed,
        installation,
    }
}

fn new_zip() -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    zip_of(&make_bundle(dir.path(), "new"))
}

fn marker(app: &Path) -> String {
    fs::read_to_string(app.join("Contents/Resources/marker")).unwrap()
}

fn is_alone(app: &Path) -> bool {
    fs::read_dir(app.parent().unwrap()).unwrap().count() == 1
}

#[test]
fn a_newer_release_replaces_the_whole_bundle() {
    let fixture = fixture("2.0.0", new_zip(), None);

    let outcome = fixture.installation.install().unwrap();

    assert_eq!(outcome, Outcome::Installed("2.0.0".to_owned()));
    assert_eq!(marker(&fixture.installed), "new");
    assert!(!fixture.installed.join("Contents/Resources/stale").exists());
    assert!(is_alone(&fixture.installed));
    verify_signature(&fixture.installed).unwrap();
    let executable = fixture.installed.join("Contents/MacOS/hiphile");
    let mode = fs::metadata(executable).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111);
}

#[test]
fn a_major_version_is_not_held_back() {
    let fixture = fixture("9.0.0", new_zip(), None);

    assert_eq!(
        fixture.installation.install().unwrap(),
        Outcome::Installed("9.0.0".to_owned())
    );
}

#[test]
fn the_same_version_changes_nothing() {
    let fixture = fixture("1.0.0", new_zip(), None);

    assert_eq!(fixture.installation.install().unwrap(), Outcome::UpToDate);
    assert_eq!(marker(&fixture.installed), "old");
}

#[test]
fn a_download_that_does_not_match_its_digest_changes_nothing() {
    let sums = format!("{}  hiphile-2.0.0-{PLATFORM}.zip\n", "0".repeat(64)).into_bytes();
    let fixture = fixture("2.0.0", new_zip(), Some(sums));

    let error = fixture.installation.install().unwrap_err();

    assert!(error.contains("damaged"), "{error}");
    assert_eq!(marker(&fixture.installed), "old");
    assert!(is_alone(&fixture.installed));
}

#[test]
fn a_bundle_whose_signature_does_not_hold_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let app = make_bundle(dir.path(), "new");
    fs::write(app.join("Contents/Resources/marker"), "tampered").unwrap();
    let fixture = fixture("2.0.0", zip_of(&app), None);

    fixture.installation.install().unwrap_err();

    assert_eq!(marker(&fixture.installed), "old");
    assert!(is_alone(&fixture.installed));
}

#[test]
fn a_release_without_this_platform_changes_nothing() {
    let mut fixture = fixture("2.0.0", new_zip(), None);
    fixture.installation.platform = "windows-x64".to_owned();

    fixture.installation.install().unwrap_err();

    assert_eq!(marker(&fixture.installed), "old");
}

#[test]
fn a_newer_version_is_found_without_touching_the_bundle() {
    let fixture = fixture("2.0.0", new_zip(), None);

    let found = fixture.installation.newer_version().unwrap();

    assert_eq!(found.as_deref(), Some("2.0.0"));
    assert_eq!(marker(&fixture.installed), "old");
}

#[test]
fn nothing_newer_is_found_at_the_same_version() {
    let fixture = fixture("1.0.0", new_zip(), None);

    assert_eq!(fixture.installation.newer_version().unwrap(), None);
}

#[test]
fn a_folder_that_cannot_change_fails_before_the_download() {
    // A zip this broken would fail with another message if it were downloaded
    let fixture = fixture("2.0.0", b"not a zip".to_vec(), None);
    let folder = fixture.installed.parent().unwrap();
    fs::set_permissions(folder, fs::Permissions::from_mode(0o555)).unwrap();

    let error = fixture.installation.install().unwrap_err();

    fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(error.contains("can't be changed"), "{error}");
    assert_eq!(marker(&fixture.installed), "old");
}

#[test]
fn no_connection_says_so() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let installation = Installation {
        api_base_url: Some(base),
        platform: PLATFORM.to_owned(),
        current_version: "1.0.0".to_owned(),
        bundle: Some(PathBuf::from("/nowhere/hiphile.app")),
    };

    let error = installation.newer_version().unwrap_err();

    assert!(error.contains("reach GitHub"), "{error}");
}
