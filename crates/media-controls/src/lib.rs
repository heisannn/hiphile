//! The system's media controls: Now Playing and the media keys on macOS, the
//! System Media Transport Controls on Windows, and MPRIS on Linux.
//!
//! Playback depends on none of it, so a failure to reach them is not reported.

use std::ffi::c_void;
use std::io::{self, Write as _};
use std::path::Path;
use std::time::{Duration, Instant};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use souvlaki::{
    MediaControlEvent, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection,
};
use subsonic::{CoverArtId, Song, SongId};
use tempfile::TempPath;

const APP_NAME: &str = "hiphile";
/// A position this far from where it should be by now is sent again, as after a
/// seek or a wait for data. Less is the usual jitter.
const LONGEST_DRIFT: Duration = Duration::from_secs(1);
/// How often a song playing has its position sent where the system does not
/// move it on by itself: MPRIS gives out the last one it was told, and Windows
/// asks for one about this often.
const POSITION_RESEND: Duration = Duration::from_secs(5);

/// What the system's media controls ask of the player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    /// By this much, or by the player's own step.
    SeekForward(Option<Duration>),
    /// By this much, or by the player's own step.
    SeekBackward(Option<Duration>),
    SeekTo(Duration),
    /// Brings the app to the front.
    Raise,
}

impl MediaCommand {
    fn from_event(event: MediaControlEvent) -> Option<Self> {
        Some(match event {
            MediaControlEvent::Play => Self::Play,
            MediaControlEvent::Pause => Self::Pause,
            MediaControlEvent::Toggle => Self::Toggle,
            MediaControlEvent::Stop => Self::Stop,
            MediaControlEvent::Next => Self::Next,
            MediaControlEvent::Previous => Self::Previous,
            MediaControlEvent::Seek(SeekDirection::Forward) => Self::SeekForward(None),
            MediaControlEvent::Seek(SeekDirection::Backward) => Self::SeekBackward(None),
            MediaControlEvent::SeekBy(SeekDirection::Forward, by) => Self::SeekForward(Some(by)),
            MediaControlEvent::SeekBy(SeekDirection::Backward, by) => Self::SeekBackward(Some(by)),
            MediaControlEvent::SetPosition(MediaPosition(position)) => Self::SeekTo(position),
            MediaControlEvent::Raise => Self::Raise,
            _ => return None,
        })
    }
}

/// How the song shown stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaStatus {
    Playing,
    Paused,
    Stopped,
}

/// A cover written where the system can read it. Dropping it removes the file.
pub struct CoverFile {
    /// Removes the file on drop.
    _path: TempPath,
    url: String,
}

impl CoverFile {
    /// Writes `image` to a file of its own. Blocks on the disk.
    ///
    /// A file rather than the cover's URL, as that carries the user's
    /// credentials, and MPRIS would show it to every program on the session bus.
    pub fn write(image: &[u8]) -> io::Result<Self> {
        let mut file = tempfile::Builder::new()
            .prefix("hiphile-cover-")
            .tempfile()?;
        file.write_all(image)?;
        let path = file.into_temp_path();
        let url = file_url(&path, cfg!(windows))?;
        Ok(Self { _path: path, url })
    }
}

/// The URL the media controls open `path` by. The Windows ones take what follows
/// the scheme as a path as it stands.
fn file_url(path: &Path, is_windows: bool) -> io::Result<String> {
    if is_windows {
        return Ok(format!("file://{}", path.display()));
    }
    url::Url::from_file_path(path)
        .map(String::from)
        .map_err(|()| io::Error::from(io::ErrorKind::InvalidFilename))
}

/// Where the system shows the song, which is not there for tests.
trait Surface {
    /// Whether it went through.
    fn set_metadata(&mut self, metadata: MediaMetadata<'_>) -> bool;
    fn set_playback(&mut self, playback: MediaPlayback);
}

impl Surface for souvlaki::MediaControls {
    fn set_metadata(&mut self, metadata: MediaMetadata<'_>) -> bool {
        self.set_metadata(metadata).is_ok()
    }

    fn set_playback(&mut self, playback: MediaPlayback) {
        self.set_playback(playback).ok();
    }
}

/// What the system was last told.
struct Shown {
    song: Option<SongId>,
    duration: Option<Duration>,
    cover: Option<CoverArtId>,
    status: MediaStatus,
    position: Duration,
    told_at: Instant,
}

/// The song the system shows. Dropping it clears that, and the system stops
/// sending commands.
pub struct MediaControls {
    /// `None` when there are no media controls to reach.
    surface: Option<Box<dyn Surface>>,
    shown: Option<Shown>,
    cover: Option<(CoverArtId, CoverFile)>,
    /// Whether a position has to be sent again for the system to move on.
    resends_position: bool,
}

impl MediaControls {
    /// Connects on behalf of `window`, and returns the commands the system
    /// sends. Stays disconnected for a window with no native handle, as in tests,
    /// whose made-up songs must not reach the system.
    pub fn connect(window: &impl HasWindowHandle) -> (Self, async_channel::Receiver<MediaCommand>) {
        let (commands, received) = async_channel::unbounded();
        let surface = window
            .window_handle()
            .ok()
            .and_then(|handle| attach(hwnd(handle.as_raw()), commands));
        (
            Self {
                surface: surface.map(|surface| Box::new(surface) as Box<dyn Surface>),
                shown: None,
                cover: None,
                resends_position: !cfg!(target_os = "macos"),
            },
            received,
        )
    }

    /// Whether the system hears of what is shown.
    pub fn is_connected(&self) -> bool {
        self.surface.is_some()
    }

    /// Whether [`Self::set_cover`] has been given `cover`, last of all.
    pub fn has_cover(&self, cover: &CoverArtId) -> bool {
        self.cover.as_ref().is_some_and(|(id, _)| id == cover)
    }

    /// Shows `cover` with the songs it belongs to, from the next [`Self::show`].
    pub fn set_cover(&mut self, cover: CoverArtId, file: CoverFile) {
        self.cover = Some((cover, file));
    }

    /// Shows `song`, or nothing. Cheap enough to call on every refresh: the
    /// system hears of a change, and of a position it could not have worked out.
    pub fn show(
        &mut self,
        song: Option<&Song>,
        duration: Option<Duration>,
        status: MediaStatus,
        position: Duration,
    ) {
        self.show_at(song, duration, status, position, Instant::now());
    }

    fn show_at(
        &mut self,
        song: Option<&Song>,
        duration: Option<Duration>,
        status: MediaStatus,
        position: Duration,
        now: Instant,
    ) {
        let Some(surface) = &mut self.surface else {
            return;
        };
        let status = if song.is_some() {
            status
        } else {
            MediaStatus::Stopped
        };
        let cover = self
            .cover
            .as_ref()
            .filter(|(id, _)| song.and_then(Song::cover_art) == Some(id));
        let is_same_song = self.shown.as_ref().is_some_and(|shown| {
            shown.song.as_ref() == song.map(Song::id)
                && shown.duration == duration
                && shown.cover.as_ref() == cover.map(|(id, _)| id)
        });
        if !is_same_song {
            let metadata = MediaMetadata {
                title: text(song.map(Song::title)),
                artist: text(song.and_then(Song::artist)),
                album: text(song.and_then(Song::album)),
                cover_url: cover.map(|(_, file)| file.url.as_str()),
                duration,
            };
            // Windows gives up on all of it over a cover it cannot open.
            if !surface.set_metadata(metadata.clone()) && metadata.cover_url.is_some() {
                surface.set_metadata(MediaMetadata {
                    cover_url: None,
                    ..metadata
                });
            }
        }
        // New metadata takes the position with it on macOS, so it goes after.
        let is_on_course = is_same_song
            && self.shown.as_ref().is_some_and(|shown| {
                let is_resend_due = self.resends_position
                    && status == MediaStatus::Playing
                    && now.saturating_duration_since(shown.told_at) >= POSITION_RESEND;
                shown.status == status
                    && !is_resend_due
                    && shown.expected_position(now).abs_diff(position) < LONGEST_DRIFT
            });
        if is_on_course {
            return;
        }
        let progress = Some(MediaPosition(position));
        surface.set_playback(match status {
            MediaStatus::Playing => MediaPlayback::Playing { progress },
            MediaStatus::Paused => MediaPlayback::Paused { progress },
            MediaStatus::Stopped => MediaPlayback::Stopped,
        });
        self.shown = Some(Shown {
            song: song.map(|song| song.id().clone()),
            duration,
            cover: cover.map(|(id, _)| id.clone()),
            status,
            position,
            told_at: now,
        });
    }

    /// Shows nothing and removes the cover's file. For when the app quits, which
    /// may not drop this.
    pub fn clear(&mut self) {
        self.cover = None;
        self.show(None, None, MediaStatus::Stopped, Duration::ZERO);
    }
}

impl Shown {
    /// Where the system takes the song to be by `now`.
    fn expected_position(&self, now: Instant) -> Duration {
        match self.status {
            MediaStatus::Playing => self.position + now.saturating_duration_since(self.told_at),
            MediaStatus::Paused | MediaStatus::Stopped => self.position,
        }
    }
}

impl Drop for MediaControls {
    fn drop(&mut self) {
        self.clear();
    }
}

/// A text as the system should have it. Windows keeps what the song before had
/// where one is left out, so it gets a blank. Anywhere else a blank would show:
/// macOS would leave an empty entry in Now Playing when there is no song.
fn text(text: Option<&str>) -> Option<&str> {
    if cfg!(windows) {
        Some(text.unwrap_or_default())
    } else {
        text
    }
}

fn hwnd(window: RawWindowHandle) -> Option<*mut c_void> {
    match window {
        RawWindowHandle::Win32(window) => Some(window.hwnd.get() as *mut c_void),
        _ => None,
    }
}

fn attach(
    hwnd: Option<*mut c_void>,
    commands: async_channel::Sender<MediaCommand>,
) -> Option<souvlaki::MediaControls> {
    let mut controls = souvlaki::MediaControls::new(PlatformConfig {
        display_name: APP_NAME,
        dbus_name: APP_NAME,
        hwnd,
    })
    .ok()?;
    controls
        .attach(move |event| {
            if let Some(command) = MediaCommand::from_event(event) {
                commands.try_send(command).ok();
            }
        })
        .ok()?;
    Some(controls)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    #[derive(Debug, PartialEq)]
    enum Told {
        Metadata {
            title: String,
            artist: String,
            has_cover: bool,
        },
        Playback(MediaPlayback),
    }

    #[derive(Clone, Default)]
    struct Recording(Rc<RefCell<Vec<Told>>>, Rc<Cell<bool>>);

    impl Recording {
        fn take(&self) -> Vec<Told> {
            self.0.take()
        }

        /// Fails metadata with a cover from now on, as Windows does when it
        /// cannot open the file.
        fn reject_covers(&self) {
            self.1.set(true);
        }
    }

    impl Surface for Recording {
        fn set_metadata(&mut self, metadata: MediaMetadata<'_>) -> bool {
            let has_cover = metadata.cover_url.is_some();
            self.0.borrow_mut().push(Told::Metadata {
                title: metadata.title.unwrap_or_default().to_owned(),
                artist: metadata.artist.unwrap_or_default().to_owned(),
                has_cover,
            });
            !(has_cover && self.1.get())
        }

        fn set_playback(&mut self, playback: MediaPlayback) {
            self.0.borrow_mut().push(Told::Playback(playback));
        }
    }

    fn recorded() -> (MediaControls, Recording) {
        let recording = Recording::default();
        let controls = MediaControls {
            surface: Some(Box::new(recording.clone())),
            shown: None,
            cover: None,
            resends_position: false,
        };
        (controls, recording)
    }

    fn song(id: &str, cover: &str) -> Song {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": id, "artist": "Artist", "coverArt": cover,
        }))
        .unwrap()
    }

    fn metadata(title: &str, has_cover: bool) -> Told {
        Told::Metadata {
            title: title.to_owned(),
            artist: "Artist".to_owned(),
            has_cover,
        }
    }

    fn playing(seconds: u64) -> Told {
        Told::Playback(MediaPlayback::Playing {
            progress: Some(MediaPosition(Duration::from_secs(seconds))),
        })
    }

    const MINUTE: Option<Duration> = Some(Duration::from_secs(60));

    #[test]
    fn a_new_song_goes_out_with_its_position_after_it() {
        let (mut controls, recording) = recorded();
        let now = Instant::now();
        let song = song("a", "c");
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Playing,
            Duration::ZERO,
            now,
        );
        assert_eq!(recording.take(), [metadata("a", false), playing(0)]);
    }

    #[test]
    fn a_song_playing_on_course_goes_out_once() {
        let (mut controls, recording) = recorded();
        let now = Instant::now();
        let song = song("a", "c");
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Playing,
            Duration::ZERO,
            now,
        );
        recording.take();
        let later = Duration::from_secs(30);
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Playing,
            later,
            now + later,
        );
        assert_eq!(recording.take(), []);
    }

    #[test]
    fn a_song_playing_is_sent_again_where_the_system_does_not_move_it_on() {
        let (mut controls, recording) = recorded();
        controls.resends_position = true;
        let now = Instant::now();
        let song = song("a", "c");
        let seconds = Duration::from_secs;
        controls.show_at(Some(&song), MINUTE, MediaStatus::Playing, seconds(0), now);
        recording.take();
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Playing,
            seconds(4),
            now + seconds(4),
        );
        assert_eq!(recording.take(), []);
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Playing,
            seconds(5),
            now + seconds(5),
        );
        assert_eq!(recording.take(), [playing(5)]);
        // The time starts over from what was sent.
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Playing,
            seconds(9),
            now + seconds(9),
        );
        assert_eq!(recording.take(), []);

        // A song paused stays where it is, so there is nothing to send.
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Paused,
            seconds(9),
            now + seconds(9),
        );
        recording.take();
        let later = now + seconds(60);
        controls.show_at(Some(&song), MINUTE, MediaStatus::Paused, seconds(9), later);
        assert_eq!(recording.take(), []);
    }

    #[test]
    fn a_cover_that_fails_leaves_the_song_shown_without_it() {
        let (mut controls, recording) = recorded();
        recording.reject_covers();
        let now = Instant::now();
        let song = song("a", "c");
        let start = Duration::ZERO;
        controls.set_cover(CoverArtId::new("c"), CoverFile::write(b"image").unwrap());
        controls.show_at(Some(&song), MINUTE, MediaStatus::Playing, start, now);
        assert_eq!(
            recording.take(),
            [metadata("a", true), metadata("a", false), playing(0)]
        );
        // It is not tried over and over.
        controls.show_at(Some(&song), MINUTE, MediaStatus::Playing, start, now);
        assert_eq!(recording.take(), []);
    }

    #[test]
    fn a_cover_file_has_the_url_each_system_opens_it_by() {
        let windows = Path::new(r"C:\Temp\hiphile-cover-a");
        assert_eq!(
            file_url(windows, true).unwrap(),
            r"file://C:\Temp\hiphile-cover-a"
        );
        #[cfg(not(windows))]
        {
            let unix = Path::new("/tmp/a folder/hiphile-cover-a");
            assert_eq!(
                file_url(unix, false).unwrap(),
                "file:///tmp/a%20folder/hiphile-cover-a"
            );
        }
    }

    #[test]
    fn a_seek_or_a_stall_sends_only_the_position() {
        let (mut controls, recording) = recorded();
        let now = Instant::now();
        let song = song("a", "c");
        let seconds = Duration::from_secs;
        controls.show_at(Some(&song), MINUTE, MediaStatus::Playing, seconds(0), now);
        recording.take();
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Playing,
            seconds(40),
            now + seconds(1),
        );
        assert_eq!(recording.take(), [playing(40)]);
        // Waiting for data, the position stands still as time goes on.
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Playing,
            seconds(40),
            now + seconds(5),
        );
        assert_eq!(recording.take(), [playing(40)]);
    }

    #[test]
    fn a_pause_goes_out_at_once_and_then_holds_still() {
        let (mut controls, recording) = recorded();
        let now = Instant::now();
        let song = song("a", "c");
        let position = Duration::from_secs(5);
        controls.show_at(Some(&song), MINUTE, MediaStatus::Playing, position, now);
        recording.take();
        controls.show_at(Some(&song), MINUTE, MediaStatus::Paused, position, now);
        assert_eq!(
            recording.take(),
            [Told::Playback(MediaPlayback::Paused {
                progress: Some(MediaPosition(position)),
            })]
        );
        let later = now + Duration::from_secs(60);
        controls.show_at(Some(&song), MINUTE, MediaStatus::Paused, position, later);
        assert_eq!(recording.take(), []);
    }

    #[test]
    fn a_duration_learned_later_goes_out_again_with_the_position() {
        let (mut controls, recording) = recorded();
        let now = Instant::now();
        let song = song("a", "c");
        controls.show_at(Some(&song), None, MediaStatus::Playing, Duration::ZERO, now);
        recording.take();
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Playing,
            Duration::ZERO,
            now,
        );
        assert_eq!(recording.take(), [metadata("a", false), playing(0)]);
    }

    #[test]
    fn a_cover_goes_with_the_songs_it_belongs_to() {
        let (mut controls, recording) = recorded();
        let now = Instant::now();
        let (first, same_album, other_album) = (song("a", "c"), song("b", "c"), song("d", "e"));
        let start = Duration::ZERO;
        controls.show_at(Some(&first), MINUTE, MediaStatus::Playing, start, now);
        recording.take();

        controls.set_cover(CoverArtId::new("c"), CoverFile::write(b"image").unwrap());
        controls.show_at(Some(&first), MINUTE, MediaStatus::Playing, start, now);
        assert_eq!(recording.take(), [metadata("a", true), playing(0)]);

        controls.show_at(Some(&same_album), MINUTE, MediaStatus::Playing, start, now);
        assert_eq!(recording.take(), [metadata("b", true), playing(0)]);

        controls.show_at(Some(&other_album), MINUTE, MediaStatus::Playing, start, now);
        assert_eq!(recording.take(), [metadata("d", false), playing(0)]);
    }

    #[test]
    fn a_cover_file_goes_when_its_cover_does() {
        let file = CoverFile::write(b"image").unwrap();
        let path = file._path.to_path_buf();
        assert_eq!(std::fs::read(&path).unwrap(), b"image");

        let (mut controls, _recording) = recorded();
        controls.set_cover(CoverArtId::new("c"), file);
        assert!(controls.has_cover(&CoverArtId::new("c")));
        controls.clear();
        assert!(!path.exists());
    }

    #[test]
    fn no_song_stops_and_blanks_what_the_song_before_left() {
        let (mut controls, recording) = recorded();
        let now = Instant::now();
        let song = song("a", "c");
        controls.show_at(
            Some(&song),
            MINUTE,
            MediaStatus::Playing,
            Duration::ZERO,
            now,
        );
        recording.take();
        controls.show_at(None, None, MediaStatus::Playing, Duration::ZERO, now);
        assert_eq!(
            recording.take(),
            [
                Told::Metadata {
                    title: String::new(),
                    artist: String::new(),
                    has_cover: false,
                },
                Told::Playback(MediaPlayback::Stopped),
            ]
        );
        controls.show_at(None, None, MediaStatus::Stopped, Duration::ZERO, now);
        assert_eq!(recording.take(), []);
    }

    #[test]
    fn dropping_it_takes_the_song_away() {
        let (mut controls, recording) = recorded();
        let song = song("a", "c");
        controls.show(Some(&song), MINUTE, MediaStatus::Playing, Duration::ZERO);
        recording.take();
        drop(controls);
        assert_eq!(
            recording.take().last(),
            Some(&Told::Playback(MediaPlayback::Stopped))
        );
    }

    #[test]
    fn events_become_the_commands_the_player_knows() {
        let by = Duration::from_secs(5);
        for (event, command) in [
            (MediaControlEvent::Play, MediaCommand::Play),
            (MediaControlEvent::Pause, MediaCommand::Pause),
            (MediaControlEvent::Toggle, MediaCommand::Toggle),
            (MediaControlEvent::Stop, MediaCommand::Stop),
            (MediaControlEvent::Next, MediaCommand::Next),
            (MediaControlEvent::Previous, MediaCommand::Previous),
            (
                MediaControlEvent::Seek(SeekDirection::Forward),
                MediaCommand::SeekForward(None),
            ),
            (
                MediaControlEvent::Seek(SeekDirection::Backward),
                MediaCommand::SeekBackward(None),
            ),
            (
                MediaControlEvent::SeekBy(SeekDirection::Forward, by),
                MediaCommand::SeekForward(Some(by)),
            ),
            (
                MediaControlEvent::SeekBy(SeekDirection::Backward, by),
                MediaCommand::SeekBackward(Some(by)),
            ),
            (
                MediaControlEvent::SetPosition(MediaPosition(by)),
                MediaCommand::SeekTo(by),
            ),
            (MediaControlEvent::Raise, MediaCommand::Raise),
        ] {
            assert_eq!(MediaCommand::from_event(event), Some(command));
        }
        assert_eq!(MediaCommand::from_event(MediaControlEvent::Quit), None);
    }
}
