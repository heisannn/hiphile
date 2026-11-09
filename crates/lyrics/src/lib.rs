mod current;
mod matching;
mod model;
pub mod musixmatch;
pub mod petitlyrics;
pub mod timing;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;

pub use current::{CurrentLyrics, FindLyrics, Lookup, LyricsSnapshot};
use matching::says_who;
pub use model::{Lyrics, PlainLyrics, TimedLine, TimedLyrics, TimedTextElement};
#[cfg(any(test, feature = "test-support"))]
pub use musixmatch::lyrics_from_richsync_body;

const ARTIST_SEPARATORS: [char; 5] = [',', '&', '，', '＆', '、'];

/// Per provider, so that a song with a long list of artists does not take a long
/// list of requests.
const MOST_ARTISTS_ASKED_ABOUT: usize = 6;

/// Nothing here carries a token: a request's URL may hold one, and [`net::Error`]
/// leaves that out.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Network(#[from] net::Error),
    #[error("the service answered with HTTP {0}")]
    HttpStatus(u16),
    /// The service turned the request down, in its answer rather than in HTTP.
    #[error("the service answered with status {status}")]
    Api {
        status: String,
        hint: Option<String>,
    },
    #[error("the service sent a response that can't be read")]
    InvalidResponse,
    /// A token was asked for moments ago, so another is not asked for yet.
    #[error("no token can be asked for yet")]
    TokenUnavailable,
}

impl Error {
    fn is_unauthorized(&self) -> bool {
        match self {
            Self::HttpStatus(401) => true,
            Self::Api { status, .. } => status == "401",
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackQuery {
    title: String,
    artist: Option<String>,
    /// Never sent as it is: [`Self::with_each_artist`] makes it an artist to ask
    /// about.
    album_artist: Option<String>,
    album: Option<String>,
    duration: Option<Duration>,
}

impl TrackQuery {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    pub fn artist(mut self, artist: Option<&str>) -> Self {
        self.artist = artist.map(str::to_owned);
        self
    }

    pub fn album_artist(mut self, album_artist: Option<&str>) -> Self {
        self.album_artist = album_artist.map(str::to_owned);
        self
    }

    pub fn album(mut self, album: Option<&str>) -> Self {
        self.album = album.map(str::to_owned);
        self
    }

    pub fn duration(mut self, duration: Option<Duration>) -> Self {
        self.duration = duration;
        self
    }

    /// One query for each artist to ask about: the song's artist, then the album's.
    /// Each name as a whole comes before those it splits into, as it may be one
    /// artist's: "Simon & Garfunkel".
    ///
    /// Without any name that says who the song is by, this query as it is.
    fn with_each_artist(&self) -> Vec<Self> {
        let mut artists: Vec<&str> = Vec::new();
        for name in [&self.artist, &self.album_artist].into_iter().flatten() {
            let parts = name.split(ARTIST_SEPARATORS).map(str::trim);
            for artist in std::iter::once(name.as_str()).chain(parts) {
                if says_who(artist) && !artists.contains(&artist) {
                    artists.push(artist);
                }
            }
        }
        if artists.is_empty() {
            return vec![self.clone()];
        }
        artists
            .into_iter()
            .take(MOST_ARTISTS_ASKED_ABOUT)
            .map(|artist| Self {
                artist: Some(artist.to_owned()),
                ..self.clone()
            })
            .collect()
    }
}

/// An error says the service could not tell, while [`Lyrics::NotFound`] says it has
/// none.
pub trait Provider: Send + Sync {
    fn find_lyrics<'a>(&'a self, query: &'a TrackQuery) -> BoxFuture<'a, Result<Lyrics, Error>>;
}

#[derive(Clone)]
pub struct Finder {
    providers: Arc<[Box<dyn Provider>]>,
}

impl Finder {
    /// `providers` in order of preference.
    pub fn new(providers: Vec<Box<dyn Provider>>) -> Self {
        Self {
            providers: providers.into(),
        }
    }

    /// The providers an app uses: PetitLyrics, which knows more Japanese songs,
    /// and then Musixmatch, which keeps its token in the app's cache folder
    /// `cache`.
    pub fn with_cache(net: net::Client, cache: Option<&Path>) -> Self {
        Self::new(vec![
            Box::new(petitlyrics::Client::new(net.clone())),
            Box::new(musixmatch::Client::with_cache(net, cache)),
        ])
    }

    /// The first lyrics with times, or else the best of what was found, the
    /// earlier provider winning a tie. A failure only counts if nothing was found,
    /// as the lyrics may yet be there.
    ///
    /// A provider is asked about each artist until it knows the song, before the
    /// next provider is asked at all.
    pub async fn find_lyrics(&self, query: &TrackQuery) -> Result<Lyrics, Error> {
        let queries = query.with_each_artist();
        let mut best = Lyrics::NotFound;
        let mut failure = None;
        for provider in self.providers.iter() {
            match first_found(&**provider, &queries).await {
                Ok(lyrics @ Lyrics::Timed(_)) => return Ok(lyrics),
                Ok(lyrics) if preference(&lyrics) > preference(&best) => best = lyrics,
                Ok(_) => {}
                Err(error) => failure = failure.or(Some(error)),
            }
        }
        match failure {
            Some(error) if best == Lyrics::NotFound => Err(error),
            _ => Ok(best),
        }
    }
}

/// A failure ends the asking, as the next query would most likely fail in the
/// same way.
async fn first_found(provider: &dyn Provider, queries: &[TrackQuery]) -> Result<Lyrics, Error> {
    for query in queries {
        match provider.find_lyrics(query).await? {
            Lyrics::NotFound => {}
            lyrics => return Ok(lyrics),
        }
    }
    Ok(Lyrics::NotFound)
}

fn preference(lyrics: &Lyrics) -> u8 {
    match lyrics {
        Lyrics::Timed(_) => 4,
        Lyrics::Plain(_) => 3,
        Lyrics::Instrumental => 2,
        Lyrics::Restricted => 1,
        Lyrics::NotFound => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures::FutureExt as _;
    use futures::executor::block_on;

    use super::*;
    use crate::model::{RawLine, timed_lines};

    struct Fixed {
        answer: Mutex<Option<Result<Lyrics, Error>>>,
        asked: Arc<Mutex<usize>>,
    }

    fn fixed(answer: Result<Lyrics, Error>) -> (Box<dyn Provider>, Arc<Mutex<usize>>) {
        let asked = Arc::new(Mutex::new(0));
        let provider = Fixed {
            answer: Mutex::new(Some(answer)),
            asked: asked.clone(),
        };
        (Box::new(provider), asked)
    }

    impl Provider for Fixed {
        fn find_lyrics<'a>(&'a self, _: &'a TrackQuery) -> BoxFuture<'a, Result<Lyrics, Error>> {
            *self.asked.lock().unwrap() += 1;
            let answer = self.answer.lock().unwrap().take().unwrap();
            async move { answer }.boxed()
        }
    }

    fn timed(text: &str) -> Lyrics {
        let lines = timed_lines(vec![RawLine {
            start: Some(1.),
            text: Some(text.to_owned()),
            ..RawLine::default()
        }]);
        Lyrics::Timed(TimedLyrics::new(lines, None))
    }

    fn plain(text: &str) -> Lyrics {
        Lyrics::Plain(PlainLyrics::new(text, None))
    }

    fn find(answers: Vec<Result<Lyrics, Error>>) -> Result<Lyrics, Error> {
        let providers = answers.into_iter().map(|answer| fixed(answer).0).collect();
        block_on(Finder::new(providers).find_lyrics(&TrackQuery::new("a song")))
    }

    type Asked = Arc<Mutex<Vec<Option<String>>>>;

    struct KnowsArtist {
        artist: &'static str,
        lyrics: Lyrics,
        asked: Asked,
    }

    impl Provider for KnowsArtist {
        fn find_lyrics<'a>(
            &'a self,
            query: &'a TrackQuery,
        ) -> BoxFuture<'a, Result<Lyrics, Error>> {
            self.asked.lock().unwrap().push(query.artist.clone());
            let is_known = query.artist.as_deref() == Some(self.artist);
            let lyrics = if is_known {
                self.lyrics.clone()
            } else {
                Lyrics::NotFound
            };
            async move { Ok(lyrics) }.boxed()
        }
    }

    fn knowing(artist: &'static str, lyrics: Lyrics) -> (Box<dyn Provider>, Asked) {
        let asked = Asked::default();
        let provider = KnowsArtist {
            artist,
            lyrics,
            asked: asked.clone(),
        };
        (Box::new(provider), asked)
    }

    fn by(artist: &str) -> TrackQuery {
        TrackQuery::new("a song").artist(Some(artist))
    }

    fn artists_of(query: &TrackQuery) -> Vec<Option<String>> {
        let queries = query.with_each_artist();
        queries.into_iter().map(|query| query.artist).collect()
    }

    fn named(artists: &[&str]) -> Vec<Option<String>> {
        artists
            .iter()
            .map(|&artist| Some(artist.to_owned()))
            .collect()
    }

    #[test]
    fn the_artists_a_name_splits_into_follow_it() {
        for name in [
            "One & Two, Three",
            "One,Two&Three",
            "One＆Two，Three",
            "One、Two、Three",
        ] {
            assert_eq!(artists_of(&by(name)), named(&[name, "One", "Two", "Three"]),);
        }
    }

    #[test]
    fn the_album_artist_follows_the_artists_of_the_song() {
        let query = by("One & Two").album_artist(Some("Three & Four"));
        assert_eq!(
            artists_of(&query),
            named(&["One & Two", "One", "Two", "Three & Four", "Three", "Four"]),
        );
    }

    #[test]
    fn an_album_artist_stands_in_for_a_song_without_an_artist() {
        let query = TrackQuery::new("a song").album_artist(Some("One"));
        assert_eq!(artists_of(&query), named(&["One"]));
    }

    #[test]
    fn a_name_already_asked_about_is_not_asked_about_again() {
        assert_eq!(artists_of(&by("One")), named(&["One"]));
        assert_eq!(artists_of(&by("One & One")), named(&["One & One", "One"]));
        let query = by("One & Two").album_artist(Some("Two"));
        assert_eq!(artists_of(&query), named(&["One & Two", "One", "Two"]));
    }

    #[test]
    fn a_name_that_does_not_say_who_is_not_asked_about() {
        assert_eq!(artists_of(&by("One, & ★")), named(&["One, & ★", "One"]));
        let query = by("One").album_artist(Some(""));
        assert_eq!(artists_of(&query), named(&["One"]));
        let query = by("One").album_artist(Some("Various Artists"));
        assert_eq!(artists_of(&query), named(&["One"]));
    }

    #[test]
    fn without_a_name_that_says_who_the_query_stays_as_it_is() {
        assert_eq!(artists_of(&TrackQuery::new("a song")), [None]);
        assert_eq!(artists_of(&by("★")), named(&["★"]));
    }

    #[test]
    fn a_long_list_of_artists_stops_at_the_most_to_ask_about() {
        let name = "A1, A2, A3, A4, A5, A6, A7, A8";
        assert_eq!(
            artists_of(&by(name).album_artist(Some("V.A."))),
            named(&[name, "A1", "A2", "A3", "A4", "A5"]),
        );
    }

    #[test]
    fn each_query_keeps_the_rest_of_the_details() {
        let query = by("One & Two")
            .album(Some("an album"))
            .duration(Some(Duration::from_secs(9)));
        let queries = query.with_each_artist();
        assert_eq!(queries[1], query.clone().artist(Some("One")));
        assert_eq!(queries[2], query.artist(Some("Two")));
    }

    #[test]
    fn artists_are_asked_about_until_the_song_is_found() {
        let (provider, asked) = knowing("Two", timed("words"));
        let finder = Finder::new(vec![provider]);

        let found = block_on(finder.find_lyrics(&by("One & Two & Three"))).unwrap();

        assert_eq!(found, timed("words"));
        assert_eq!(
            *asked.lock().unwrap(),
            named(&["One & Two & Three", "One", "Two"])
        );
    }

    #[test]
    fn a_song_found_under_the_whole_name_asks_nothing_more() {
        let (provider, asked) = knowing("One & Two", plain("words"));
        let finder = Finder::new(vec![provider]);

        let found = block_on(finder.find_lyrics(&by("One & Two"))).unwrap();

        assert_eq!(found, plain("words"));
        assert_eq!(*asked.lock().unwrap(), named(&["One & Two"]));
    }

    #[test]
    fn a_song_is_found_under_its_album_artist() {
        let (provider, _) = knowing("Three", plain("words"));
        let finder = Finder::new(vec![provider]);
        let query = by("One").album_artist(Some("Three"));

        assert_eq!(
            block_on(finder.find_lyrics(&query)).unwrap(),
            plain("words")
        );
    }

    #[test]
    fn an_earlier_provider_is_asked_about_each_artist_before_a_later_one_at_all() {
        let (first, _) = knowing("Two", timed("first"));
        let (second, asked) = knowing("One & Two", timed("second"));
        let finder = Finder::new(vec![first, second]);

        let found = block_on(finder.find_lyrics(&by("One & Two"))).unwrap();

        assert_eq!(found, timed("first"));
        assert!(asked.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failure_is_not_repeated_for_each_artist() {
        let (provider, asked) = fixed(Err(Error::InvalidResponse));
        let finder = Finder::new(vec![provider]);

        let found = block_on(finder.find_lyrics(&by("One & Two")));

        assert!(matches!(found, Err(Error::InvalidResponse)), "{found:?}");
        assert_eq!(*asked.lock().unwrap(), 1);
    }

    #[test]
    fn lyrics_with_times_end_the_search() {
        let (first, _) = fixed(Ok(timed("first")));
        let (second, asked) = fixed(Ok(timed("second")));

        let finder = Finder::new(vec![first, second]);
        let found = block_on(finder.find_lyrics(&TrackQuery::new("a song"))).unwrap();

        assert_eq!(found, timed("first"));
        assert_eq!(*asked.lock().unwrap(), 0);
    }

    #[test]
    fn times_from_a_later_provider_beat_words_from_an_earlier_one() {
        let found = find(vec![Ok(plain("words")), Ok(timed("times"))]).unwrap();
        assert_eq!(found, timed("times"));
    }

    #[test]
    fn the_earlier_provider_wins_a_tie() {
        let found = find(vec![Ok(plain("first")), Ok(plain("second"))]).unwrap();
        assert_eq!(found, plain("first"));
    }

    #[test]
    fn nothing_from_one_provider_leaves_it_to_the_next() {
        let found = find(vec![Ok(Lyrics::NotFound), Ok(plain("words"))]).unwrap();
        assert_eq!(found, plain("words"));
    }

    #[test]
    fn a_failure_does_not_hide_what_another_provider_found() {
        let found = find(vec![Err(Error::InvalidResponse), Ok(plain("words"))]).unwrap();
        assert_eq!(found, plain("words"));
    }

    #[test]
    fn a_failure_is_not_taken_for_there_being_no_lyrics() {
        let found = find(vec![Ok(Lyrics::NotFound), Err(Error::TokenUnavailable)]);
        assert!(matches!(found, Err(Error::TokenUnavailable)), "{found:?}");
    }

    #[test]
    fn the_first_failure_is_the_one_told() {
        let found = find(vec![
            Err(Error::HttpStatus(500)),
            Err(Error::InvalidResponse),
        ]);
        assert!(matches!(found, Err(Error::HttpStatus(500))), "{found:?}");
    }

    #[test]
    fn no_lyrics_anywhere_is_an_answer() {
        let found = find(vec![Ok(Lyrics::NotFound), Ok(Lyrics::NotFound)]).unwrap();
        assert_eq!(found, Lyrics::NotFound);
    }
}
