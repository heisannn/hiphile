use std::collections::HashSet;

use chrono::{DateTime, Local, TimeDelta, Utc};
use futures::future::{join_all, try_join4};
use subsonic::{Album, AlbumId, Song};

/// How many to pick, for the widest window.
const RECOMMENDED: usize = 12;
/// How long an album stays out of the picks after being played.
const HEARD_DAYS: i64 = 14;
/// How many of the albums played last to look through for those.
const HEARD_ALBUMS: u32 = 100;
/// How many albums to start from for similar songs. Each brings a few artists
/// at most, so a single one would not fill the shelf.
const SEEDS: usize = 3;
/// How many albums to try for that, as a server may have nothing on any.
const SEEDS_TRIED: usize = 6;
/// How many similar songs to ask of each. Most are by a few artists.
const SIMILAR_SONGS: u32 = 50;
/// The fewest similar songs worth using an album's. Fewer say little about
/// what is similar, and leave the shelf to the next album.
const MIN_SIMILAR_SONGS: usize = 5;
/// How many of the most played and of the random albums to fill up with.
const FILL_ALBUMS: u32 = 30;

/// The local day, which is how long a pick lasts.
pub fn today() -> String {
    Local::now().date_naive().to_string()
}

#[non_exhaustive]
pub struct Recommendations {
    pub albums: Vec<Album>,
    /// Whether everything asked of the server came through. Otherwise this is
    /// not what the day's pick should be, though it may be better than nothing.
    pub complete: bool,
}

impl Recommendations {
    pub fn new(albums: Vec<Album>, complete: bool) -> Self {
        Self { albums, complete }
    }
}

/// Picks the albums to recommend now. Similar songs are the best guide but
/// depend on the server's outside service, so a failure there only leaves more
/// to the other albums.
pub async fn fetch_recommended(
    client: &client::Client,
    new_albums: u32,
    most_played: u32,
) -> Result<Recommendations, client::Error> {
    let (played, newest, frequent, random) = try_join4(
        client.recently_played_albums(HEARD_ALBUMS),
        client.newest_albums(new_albums),
        client.frequent_albums(most_played + FILL_ALBUMS),
        client.random_albums(FILL_ALBUMS),
    )
    .await?;

    let (most_played, frequent) = frequent.split_at(frequent.len().min(most_played as usize));
    let similar = similar_songs(&played, |seed| client.similar_songs(seed, SIMILAR_SONGS)).await;

    let picks = pick(
        &Sources {
            similar: &similar.lists,
            frequent,
            random: &random,
            played: &played,
            newest: &newest,
            most_played,
            now: Utc::now(),
        },
        RECOMMENDED,
    );
    // A song names its album, but not who the album as a whole is by
    let loaded = join_all(picks.iter().map(|album| client.album(album))).await;
    let mut albums = Vec::new();
    let mut failure = None;
    for album in loaded {
        match album {
            Ok(album) => albums.push(album.into_parts().0),
            Err(error) => failure = Some(error),
        }
    }
    match failure {
        Some(error) if albums.is_empty() => Err(error),
        failure => Ok(Recommendations {
            albums: one_per_artist(albums.iter()).cloned().collect(),
            complete: similar.complete && failure.is_none(),
        }),
    }
}

/// Similar songs, per album they are similar to.
struct SimilarSongs {
    lists: Vec<Vec<Song>>,
    /// Whether a failure can have cost the pick a list.
    complete: bool,
}

/// Asks for similar songs to albums among those played last, by different
/// artists, leaving out those that find too few. Each answer takes the server
/// several seconds, so they are asked for at once, with more only if some fell
/// short.
async fn similar_songs<'a, Ask, Answer>(played: &'a [Album], ask: Ask) -> SimilarSongs
where
    Ask: Fn(&'a AlbumId) -> Answer,
    Answer: Future<Output = Result<Vec<Song>, client::Error>>,
{
    let seeds: Vec<_> = one_per_artist(played.iter()).take(SEEDS_TRIED).collect();
    let mut lists = Vec::new();
    let mut failed = false;
    for seeds in seeds.chunks(SEEDS) {
        for answer in join_all(seeds.iter().map(|seed| ask(seed.id()))).await {
            match answer {
                Ok(songs) if songs.len() >= MIN_SIMILAR_SONGS => lists.push(songs),
                Ok(_) => {}
                Err(_) => failed = true,
            }
        }
        if lists.len() >= SEEDS {
            break;
        }
    }
    let complete = !failed || lists.len() >= SEEDS;
    lists.truncate(SEEDS);
    SimilarSongs { lists, complete }
}

struct Sources<'a> {
    /// Similar songs, per album they are similar to.
    similar: &'a [Vec<Song>],
    frequent: &'a [Album],
    random: &'a [Album],
    played: &'a [Album],
    newest: &'a [Album],
    most_played: &'a [Album],
    now: DateTime<Utc>,
}

#[derive(Clone, Copy)]
struct Candidate<'a> {
    album: &'a AlbumId,
    artist: Option<&'a str>,
}

/// Up to `count` albums, those with similar songs first and the most played
/// and random ones for the rest, taking turns between sources so that none
/// outweighs the others. Leaves out albums played lately and those on the other
/// shelves, and takes one album per artist.
fn pick(sources: &Sources<'_>, count: usize) -> Vec<AlbumId> {
    let similar: Vec<Vec<_>> = sources
        .similar
        .iter()
        .map(|songs| songs.iter().filter_map(song_candidate).collect())
        .collect();
    let others: [Vec<_>; 2] = [sources.frequent, sources.random]
        .map(|albums| albums.iter().map(album_candidate).collect());

    let heard_since = sources.now - TimeDelta::days(HEARD_DAYS);
    let mut taken: HashSet<&AlbumId> = sources
        .played
        .iter()
        .filter(|album| played_since(album, heard_since))
        .chain(sources.newest)
        .chain(sources.most_played)
        .map(Album::id)
        .collect();
    let mut artists = HashSet::new();
    let mut picks = Vec::new();
    for candidate in taking_turns(&similar).chain(taking_turns(&others)) {
        if picks.len() == count {
            break;
        }
        if taken.contains(candidate.album)
            || candidate
                .artist
                .is_some_and(|artist| !artists.insert(artist))
        {
            continue;
        }
        taken.insert(candidate.album);
        picks.push(candidate.album.clone());
    }
    picks
}

fn taking_turns<'a, T: Copy>(lists: &'a [Vec<T>]) -> impl Iterator<Item = T> + 'a {
    let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
    (0..longest).flat_map(move |ix| lists.iter().filter_map(move |list| list.get(ix).copied()))
}

fn song_candidate(song: &Song) -> Option<Candidate<'_>> {
    Some(Candidate {
        album: song.album_id()?,
        artist: song
            .artist_id()
            .map(|artist| artist.as_str())
            .or(song.artist()),
    })
}

fn album_candidate(album: &Album) -> Candidate<'_> {
    Candidate {
        album: album.id(),
        artist: album_artist(album),
    }
}

/// Who an album counts as by: the server's ID for them, or else their name.
fn album_artist(album: &Album) -> Option<&str> {
    album
        .artist_id()
        .map(|artist| artist.as_str())
        .or(album.artist())
}

/// Albums by no known artist all stay.
fn one_per_artist<'a>(albums: impl Iterator<Item = &'a Album>) -> impl Iterator<Item = &'a Album> {
    let mut artists = HashSet::new();
    albums.filter(move |album| album_artist(album).is_none_or(|artist| artists.insert(artist)))
}

/// A time that cannot be read says nothing, so it does not keep the album out.
fn played_since(album: &Album, since: DateTime<Utc>) -> bool {
    album
        .played()
        .and_then(|played| DateTime::parse_from_rfc3339(played).ok())
        .is_some_and(|played| played > since)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use futures::executor::block_on;
    use serde_json::{Value, json};

    use super::*;

    fn song(album: &str, artist: &str) -> Song {
        serde_json::from_value(json!({
            "id": format!("{album}-song"), "title": "t", "albumId": album, "artistId": artist,
        }))
        .unwrap()
    }

    fn album_from(fields: Value) -> Album {
        serde_json::from_value(fields).unwrap()
    }

    fn album(id: &str, artist: &str) -> Album {
        album_from(json!({ "id": id, "name": id, "artistId": artist }))
    }

    fn played(id: &str, at: &str) -> Album {
        album_from(json!({ "id": id, "name": id, "played": at }))
    }

    fn now() -> DateTime<Utc> {
        "2026-11-04T12:00:00Z".parse().unwrap()
    }

    fn sources<'a>(similar: &'a [Vec<Song>], frequent: &'a [Album]) -> Sources<'a> {
        Sources {
            similar,
            frequent,
            random: &[],
            played: &[],
            newest: &[],
            most_played: &[],
            now: now(),
        }
    }

    fn ids(picks: &[AlbumId]) -> Vec<&str> {
        picks.iter().map(AlbumId::as_str).collect()
    }

    #[test]
    fn similar_albums_take_turns_and_come_before_the_rest() {
        let similar = [
            vec![song("a1", "A"), song("a2", "B")],
            vec![song("b1", "C"), song("b2", "D")],
        ];
        let frequent = [album("f", "E")];
        assert_eq!(
            ids(&pick(&sources(&similar, &frequent), 12)),
            ["a1", "b1", "a2", "b2", "f"]
        );
    }

    #[test]
    fn the_rest_take_turns_too() {
        let frequent = [album("f1", "A"), album("f2", "B")];
        let random = [album("r1", "C"), album("r2", "D")];
        let sources = Sources {
            random: &random,
            ..sources(&[], &frequent)
        };
        assert_eq!(ids(&pick(&sources, 12)), ["f1", "r1", "f2", "r2"]);
    }

    #[test]
    fn stops_at_the_count() {
        let frequent = [album("f1", "A"), album("f2", "B")];
        assert_eq!(ids(&pick(&sources(&[], &frequent), 1)), ["f1"]);
    }

    #[test]
    fn one_album_per_artist_across_every_source() {
        let similar = [vec![song("a1", "A"), song("a2", "A"), song("b1", "B")]];
        let frequent = [album("f", "A"), album("g", "C")];
        assert_eq!(
            ids(&pick(&sources(&similar, &frequent), 12)),
            ["a1", "b1", "g"]
        );
    }

    #[test]
    fn songs_of_one_album_make_one_pick() {
        let similar = [vec![song("a1", "A"), song("a1", "B")]];
        assert_eq!(ids(&pick(&sources(&similar, &[]), 12)), ["a1"]);
    }

    #[test]
    fn albums_with_no_known_artist_are_not_taken_for_one_artist() {
        let frequent = [
            album_from(json!({ "id": "x", "name": "x" })),
            album_from(json!({ "id": "y", "name": "y" })),
        ];
        assert_eq!(ids(&pick(&sources(&[], &frequent), 12)), ["x", "y"]);
    }

    #[test]
    fn a_song_with_no_album_is_passed_over() {
        let loose = serde_json::from_value(json!({ "id": "s", "title": "t" })).unwrap();
        let similar = [vec![loose, song("a1", "A")]];
        assert_eq!(ids(&pick(&sources(&similar, &[]), 12)), ["a1"]);
    }

    #[test]
    fn albums_played_in_the_last_two_weeks_are_left_out() {
        let frequent = [
            album("lately", "A"),
            album("long-ago", "B"),
            album("unreadable", "C"),
        ];
        let played_albums = [
            played("lately", "2026-10-21T12:00:00.25Z"),
            played("long-ago", "2026-10-21T11:59:59.25Z"),
            played("unreadable", "yesterday"),
        ];
        let sources = Sources {
            played: &played_albums,
            ..sources(&[], &frequent)
        };
        assert_eq!(ids(&pick(&sources, 12)), ["long-ago", "unreadable"]);
    }

    #[test]
    fn an_album_left_out_does_not_use_up_its_artist() {
        let frequent = [album("lately", "A"), album("other", "A")];
        let played_albums = [played("lately", "2026-11-03T12:00:00Z")];
        let sources = Sources {
            played: &played_albums,
            ..sources(&[], &frequent)
        };
        assert_eq!(ids(&pick(&sources, 12)), ["other"]);
    }

    #[test]
    fn new_albums_are_left_out() {
        let frequent = [album("new", "A"), album("old", "B")];
        let newest = [album("new", "A")];
        let sources = Sources {
            newest: &newest,
            ..sources(&[], &frequent)
        };
        assert_eq!(ids(&pick(&sources, 12)), ["old"]);
    }

    #[test]
    fn albums_on_the_most_played_shelf_are_left_out() {
        let similar = [vec![song("top", "A"), song("other", "B")]];
        let most_played = [album("top", "A")];
        let sources = Sources {
            most_played: &most_played,
            ..sources(&similar, &[])
        };
        assert_eq!(ids(&pick(&sources, 12)), ["other"]);
    }

    #[test]
    fn a_library_with_no_plays_still_gets_picks() {
        let random = [album("r", "A")];
        let sources = Sources {
            random: &random,
            ..sources(&[], &[])
        };
        assert_eq!(ids(&pick(&sources, 12)), ["r"]);
    }

    #[test]
    fn the_first_album_of_each_artist_stays() {
        let albums = [album("a1", "A"), album("a2", "A"), album("b1", "B")];
        let kept: Vec<_> = one_per_artist(albums.iter())
            .map(|album| album.id().as_str())
            .collect();
        assert_eq!(kept, ["a1", "b1"]);
    }

    /// Runs [`similar_songs`] over albums by different artists, with `answers`
    /// as how many songs each finds, in order. Says which were asked.
    fn similar_to(answers: &[Result<usize, ()>]) -> (SimilarSongs, Vec<String>) {
        let played: Vec<_> = (0..answers.len())
            .map(|ix| album(&format!("{ix}"), &format!("artist-{ix}")))
            .collect();
        let asked = RefCell::new(Vec::new());
        let found = block_on(similar_songs(&played, |seed: &AlbumId| {
            asked.borrow_mut().push(seed.to_string());
            let answer = answers[seed.as_str().parse::<usize>().unwrap()];
            async move {
                match answer {
                    Ok(count) => Ok(vec![song("a", "x"); count]),
                    Err(()) => Err(client::Error::HttpStatus(500)),
                }
            }
        }));
        (found, asked.into_inner())
    }

    fn lengths(found: &SimilarSongs) -> Vec<usize> {
        found.lists.iter().map(Vec::len).collect()
    }

    #[test]
    fn three_albums_with_enough_songs_need_no_more_asked() {
        let (found, asked) = similar_to(&[Ok(5), Ok(6), Ok(7), Ok(8)]);
        assert_eq!(lengths(&found), [5, 6, 7]);
        assert_eq!(asked, ["0", "1", "2"]);
        assert!(found.complete);
    }

    #[test]
    fn an_album_with_too_few_songs_leaves_its_place_to_the_next() {
        let (found, asked) = similar_to(&[Ok(4), Ok(5), Ok(6), Ok(7)]);
        assert_eq!(lengths(&found), [5, 6, 7]);
        assert_eq!(asked, ["0", "1", "2", "3"]);
        assert!(found.complete);
    }

    #[test]
    fn no_more_than_three_lists_come_of_more_answers() {
        let (found, _) = similar_to(&[Ok(0), Ok(5), Ok(6), Ok(7), Ok(8), Ok(9)]);
        assert_eq!(lengths(&found), [5, 6, 7]);
    }

    #[test]
    fn no_more_than_six_albums_are_asked() {
        let (found, asked) = similar_to(&[Ok(0); 8]);
        assert!(found.lists.is_empty());
        assert_eq!(asked.len(), 6);
        assert!(found.complete);
    }

    #[test]
    fn one_album_per_artist_is_asked() {
        let played = [album("a1", "A"), album("a2", "A"), album("b1", "B")];
        let asked = RefCell::new(Vec::new());
        block_on(similar_songs(&played, |seed: &AlbumId| {
            asked.borrow_mut().push(seed.to_string());
            async { Ok(Vec::new()) }
        }));
        assert_eq!(asked.into_inner(), ["a1", "b1"]);
    }

    #[test]
    fn a_failure_that_leaves_lists_short_makes_the_answer_incomplete() {
        let (found, _) = similar_to(&[Err(()), Ok(5)]);
        assert_eq!(lengths(&found), [5]);
        assert!(!found.complete);
    }

    #[test]
    fn a_failure_others_made_up_for_does_no_harm() {
        let (found, _) = similar_to(&[Err(()), Ok(5), Ok(6), Ok(7)]);
        assert_eq!(lengths(&found), [5, 6, 7]);
        assert!(found.complete);
    }
}
