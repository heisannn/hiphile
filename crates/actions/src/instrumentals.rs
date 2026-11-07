//! Whether instrumental songs are left out of the queue. Servers say nothing of
//! a song's vocals, so a song's title is all there is to go by.

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::{Action, App, Global};
use subsonic::Song;

/// In lowercase, with spaces single. They mean the same wherever they are in a title.
const NAMES: [&str; 17] = [
    "instrumental",
    "インストゥルメンタル",
    "インストルメンタル",
    "off vocal",
    "off-vocal",
    "offvocal",
    "オフボーカル",
    "オフ・ボーカル",
    "オフヴォーカル",
    "オフ・ヴォーカル",
    "without vocal",
    "no vocal",
    "ボーカルレス",
    "オリジナル・カラオケ",
    "オリジナルカラオケ",
    "bgm ver",
    "bgm version",
];

/// In lowercase. Each also makes up other words and titles, such as "Installation",
/// "インストール" and "カラオケ行こ!", so it has to stand apart from them.
const MARKS: [&str; 6] = ["inst", "karaoke", "カラオケ", "ｶﾗｵｹ", "インスト", "ｲﾝｽﾄ"];

/// In lowercase. Ordinary words too, as in "The Instrument" and "T-Minus One", so
/// only a mark set off on both sides counts.
const ENCLOSED_MARKS: [&str; 3] = ["instrument", "minus one", "backing track"];

const OPENINGS: &str = "([{<（［【〈《＜「『〜～~-－/／";
const CLOSINGS: &str = ")]}>）］】〉》＞」』〜～~-－";

pub fn is_instrumental(title: &str) -> bool {
    let title = title
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    NAMES.iter().any(|name| has_name(&title, name))
        || MARKS.iter().any(|mark| has_mark(&title, mark))
        || ENCLOSED_MARKS
            .iter()
            .any(|mark| has_enclosed_mark(&title, mark))
}

fn is_ascii_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric())
}

fn has_name(title: &str, name: &str) -> bool {
    title.match_indices(name).any(|(ix, _)| {
        let rest = &title[ix + name.len()..];
        // A plural is the same name: "no vocals"
        let rest = rest.strip_prefix('s').unwrap_or(rest);
        !is_ascii_word(title[..ix].chars().next_back()) && !is_ascii_word(rest.chars().next())
    })
}

fn has_mark(title: &str, mark: &str) -> bool {
    title.match_indices(mark).any(|(ix, _)| {
        let rest = after_mark(&title[ix + mark.len()..]);
        match title[..ix].chars().next_back() {
            None => says_version(rest),
            Some(before) => {
                (before == ' ' || OPENINGS.contains(before))
                    && (rest.is_empty() || closes(rest) || says_version(rest))
            }
        }
    })
}

fn has_enclosed_mark(title: &str, mark: &str) -> bool {
    title.match_indices(mark).any(|(ix, _)| {
        let rest = after_mark(&title[ix + mark.len()..]);
        title[..ix]
            .chars()
            .next_back()
            .is_some_and(|before| OPENINGS.contains(before))
            && (closes(rest) || says_version(rest))
    })
}

fn after_mark(rest: &str) -> &str {
    rest.trim_start_matches(['.', ' ', '・'])
}

/// A dash goes between the parts of a word too, as in "Mr-Karaoke-Man".
fn closes(rest: &str) -> bool {
    let mut chars = rest.chars();
    chars.next().is_some_and(|c| CLOSINGS.contains(c))
        && !chars.next().is_some_and(char::is_alphanumeric)
}

fn says_version(rest: &str) -> bool {
    rest.starts_with("バージョン")
        || ["version", "ver", "arrange"].iter().any(|word| {
            rest.strip_prefix(word)
                .is_some_and(|rest| !is_ascii_word(rest.chars().next()))
        })
}

#[derive(Default)]
struct SkipInstrumentals(bool);

impl Global for SkipInstrumentals {}

/// Every window draws again, as any of them may show the songs this greys out.
pub fn set_skip_instrumentals(is_on: bool, cx: &mut App) {
    cx.set_global(SkipInstrumentals(is_on));
    cx.refresh_windows();
}

pub fn is_skipped(song: &Song, cx: &App) -> bool {
    cx.try_global::<SkipInstrumentals>()
        .is_some_and(|skip| skip.0)
        && is_instrumental(song.title())
}

pub fn without_skipped(songs: &[Song], cx: &App) -> Vec<Song> {
    songs
        .iter()
        .filter(|song| !is_skipped(song, cx))
        .cloned()
        .collect()
}

pub fn queue_menu_items(
    menu: PopupMenu,
    song: &Song,
    play: impl Action,
    play_next: impl Action,
    add_to_queue: impl Action,
    cx: &App,
) -> PopupMenu {
    let is_skipped = is_skipped(song, cx);
    menu.menu_with_disabled("Play", Box::new(play), is_skipped)
        .menu_with_disabled("Play Next", Box::new(play_next), is_skipped)
        .menu_with_disabled("Add to Queue", Box::new(add_to_queue), is_skipped)
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use serde_json::json;

    use super::*;

    #[test]
    fn titles_that_mark_a_song_as_instrumental_do() {
        for title in [
            "Believe [instrumental]",
            "1000回潤んだ空 -instrumental-",
            "Agent CODE:BLUE-Instrumental",
            "360°〜instrumental〜",
            "Beautiful World (Da Capo Version [Instrumental])",
            "Cagayake!GIRLS [5人Ver.] [instrumental] [bass]",
            "ASEED (Off Vocal)",
            "Blooming Lily (Off Vocal Ver.)",
            "いえないや(オフボーカル)",
            "#HE4DSHOT (オリジナル・カラオケ)",
            "Affection (オリジナルカラオケ)",
            "無限回廊　カラオケ",
            "Are You Champion? Yeah!! I'm Champion !!(カラオケver.)",
            "Star Map karaoke version",
            "Be with (inst.)",
            "EDEN inst",
            "Blumenkranz：[nZk] ver. <INST>",
            "櫻ノ詩 -Piano inst ver-",
            "Blue sky <Instrument Version>",
            "アルカテイル -Instrument-",
            "just for you (インスト)",
            "アレセイア (Inst Arrange ver.)",
            "-影二つ- -インストゥルメンタル-",
            "HL pinch no vocals 2023",
            "Song (Off　Vocal)",
            "Song (Off  Vocal)",
            "Song (オフ・ヴォーカル)",
            "Song（ｶﾗｵｹ）",
            "Song (ｲﾝｽﾄ)",
            "Karaoke Version",
            "カラオケバージョン",
            "インスト・バージョン",
            "Song (Minus One)",
            "Song [Backing Track]",
            "GOIN'!!! (BGM Version) (C21)",
        ] {
            assert!(is_instrumental(title), "{title}");
        }
    }

    #[test]
    fn titles_that_only_look_alike_do_not() {
        for title in [
            "Installation",
            "HAYAMInst. (2016.11.03 @品川ステラボール)",
            "rEiNsTaLL",
            "Love install",
            "アンインストール",
            "インストップデート",
            "カラオケ行こ!",
            "櫻ノ詩 -Piano Vocal ver-",
            "空気力学少女と少年の詩 -PianoVocalVar.-",
            "Catch You Catch Me (TV Size)",
            "BGM",
            "Without You",
            "Fly with Me (Vocal)",
            "The Instrumentalist",
            "Human Instrumentality",
            "No Vocalist",
            "Backing Tracks of My Life",
            "BGM Verse",
            "Minus One",
            "T-Minus One",
            "Ten Minus One",
            "My Karaoke Verse",
            "The Instrument Very Loud",
            "Tell The Inst Versus",
            "Mr-Karaoke-Man",
            "Anti-Karaoke-Club",
            "The Instrument",
            "Karaoke",
            "カラオケ",
        ] {
            assert!(!is_instrumental(title), "{title}");
        }
    }

    fn song(title: &str) -> Song {
        serde_json::from_value(json!({ "id": title, "title": title })).unwrap()
    }

    #[gpui_kit::test]
    fn only_instrumentals_are_skipped_and_only_while_it_is_on(cx: &mut TestAppContext) {
        let vocal = song("Believe");
        let instrumental = song("Believe [instrumental]");
        let songs = [vocal.clone(), instrumental.clone()];
        cx.update(|cx| {
            assert!(!is_skipped(&instrumental, cx));
            assert_eq!(without_skipped(&songs, cx), songs);

            set_skip_instrumentals(true, cx);
            assert!(is_skipped(&instrumental, cx));
            assert!(!is_skipped(&vocal, cx));
            assert_eq!(without_skipped(&songs, cx), std::slice::from_ref(&vocal));

            set_skip_instrumentals(false, cx);
            assert!(!is_skipped(&instrumental, cx));
        });
    }
}
