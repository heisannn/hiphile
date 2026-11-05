use std::path::PathBuf;
use std::time::Duration;

use self_update::ReleaseAsset;
use self_update::backends::github::Update;
use self_update::errors::Error;
use self_update::update::UpdateStrategy;

const REPO_OWNER: &str = "heisannn";
const REPO_NAME: &str = "hiphile";
const BIN_NAME: &str = "hiphile";
const BUNDLE_IN_ARCHIVE: &str = "hiphile.app";
const SUMS_ASSET: &str = "SHA256SUMS";
/// A stalled connection would otherwise keep the app from looking for updates until it quits.
const TIMEOUT: Duration = Duration::from_secs(600);

/// The part of a release asset's name that `.github/workflows/release.yml` gives this platform.
const PLATFORM: Option<&str> = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("macos-arm64")
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Some("macos-x64")
} else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Some("windows-x64")
} else {
    None
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    UpToDate,
    Installed(String),
}

/// Errors are the words to show the user.
pub(crate) trait Releases: Send + Sync + 'static {
    fn newer_version(&self) -> Result<Option<String>, String>;
    fn install(&self) -> Result<Outcome, String>;
}

#[derive(Clone, Debug)]
pub(crate) struct Installation {
    /// `None` for GitHub itself.
    api_base_url: Option<String>,
    platform: String,
    current_version: String,
    /// `None` for the bundle the running executable is in.
    bundle: Option<PathBuf>,
}

impl Installation {
    /// `None` on a platform that has no releases.
    pub(crate) fn of_this_app() -> Option<Self> {
        Some(Self {
            api_base_url: None,
            platform: PLATFORM?.to_owned(),
            current_version: crate::VERSION.to_owned(),
            bundle: None,
        })
    }

    fn updater(&self) -> Result<Update, Error> {
        let mut updater = Update::configure();
        updater
            .repo_owner(REPO_OWNER)
            .repo_name(REPO_NAME)
            .bin_name(BIN_NAME)
            .current_version(self.current_version.clone())
            .update_strategy(UpdateStrategy::Latest)
            .asset_matcher(matcher(&self.platform))
            .checksum_from_asset(SUMS_ASSET)
            .timeout(TIMEOUT)
            .check_install_path_writable(true)
            .no_confirm(true)
            .show_output(false)
            .show_download_progress(false);
        if let Some(url) = &self.api_base_url {
            updater.api_base_url(url.clone());
        }
        if cfg!(target_os = "macos") {
            updater
                .bundle_path_in_archive(BUNDLE_IN_ARCHIVE)
                .verify_binary(verify_signature);
        }
        if let Some(bundle) = &self.bundle {
            updater.bundle_install_path(bundle);
        }
        updater.build()
    }
}

impl Releases for Installation {
    fn newer_version(&self) -> Result<Option<String>, String> {
        let newest = self
            .updater()
            .and_then(|updater| updater.is_update_available())
            .map_err(|error| explain(&error))?;
        Ok(newest.map(|release| release.version().to_owned()))
    }

    fn install(&self) -> Result<Outcome, String> {
        let status = self
            .updater()
            .and_then(|updater| updater.update())
            .map_err(|error| explain(&error))?;
        Ok(if status.is_updated() {
            Outcome::Installed(status.version().to_owned())
        } else {
            Outcome::UpToDate
        })
    }
}

fn explain(error: &Error) -> String {
    match error {
        Error::InstallPathNotWritable { .. } => {
            "Couldn't install the update, as hiphile's folder can't be changed. Move hiphile to \
             a folder you can change, then try again."
                .to_owned()
        }
        Error::AppTranslocated { .. } => {
            "Couldn't install the update from where macOS is running hiphile. Move hiphile to \
             Applications, then open it again."
                .to_owned()
        }
        Error::Transport(_) => {
            "Couldn't reach GitHub. Check your connection, then try again.".to_owned()
        }
        Error::RateLimited { .. } => {
            "GitHub is limiting requests from this network. Try again later.".to_owned()
        }
        Error::NoReleaseFound { .. } => {
            "The newest version has no build for this system yet. Try again later.".to_owned()
        }
        Error::ChecksumMismatch { .. }
        | Error::ChecksumSourceInvalid { .. }
        | Error::VerificationRejected { .. }
        | Error::Zip(_) => {
            "The downloaded update is damaged, so it wasn't installed. Try again later.".to_owned()
        }
        other => format!("Couldn't update: {other}."),
    }
}

/// Picks the zip of `platform`: a macOS release also holds a disk image.
fn matcher(
    platform: &str,
) -> impl Fn(&[ReleaseAsset]) -> Option<ReleaseAsset> + Send + Sync + use<> {
    let suffix = format!("-{platform}.zip");
    move |assets| {
        assets
            .iter()
            .find(|asset| asset.name().ends_with(&suffix))
            .cloned()
    }
}

/// macOS kills an app whose signature does not hold, which could then not update itself again.
fn verify_signature(bundle: &std::path::Path) -> Result<(), Error> {
    let output = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(Error::verification_rejected(
        String::from_utf8_lossy(&output.stderr).trim(),
    ))
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
