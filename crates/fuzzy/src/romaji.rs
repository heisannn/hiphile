//! One spelling for every way of writing a reading in Latin letters, so that
//! what is typed and what a title reads as meet whichever system each follows.

/// Whether `c` can be part of a reading: a Latin letter, a digit or kana.
pub(crate) fn is_readable(c: char) -> bool {
    c.is_ascii_alphanumeric() || is_kana(c)
}

/// Whether `c` is hiragana, katakana or the mark that lengthens a vowel.
pub(crate) fn is_kana(c: char) -> bool {
    matches!(c, 'ぁ'..='ゖ' | 'ァ'..='ヺ' | 'ー')
}

/// Katakana in place of hiragana, with everything else as it is.
pub(crate) fn to_katakana(c: char) -> char {
    if matches!(c, 'ぁ'..='ゖ') {
        char::from_u32(c as u32 + 0x60).unwrap_or(c)
    } else {
        c
    }
}

/// The Hepburn romaji of a syllable written as one kana.
fn syllable(kana: char) -> Option<&'static str> {
    Some(match kana {
        'ア' | 'ァ' => "a",
        'イ' | 'ィ' => "i",
        'ウ' | 'ゥ' => "u",
        'エ' | 'ェ' => "e",
        'オ' | 'ォ' => "o",
        'カ' => "ka",
        'キ' => "ki",
        'ク' => "ku",
        'ケ' => "ke",
        'コ' => "ko",
        'ガ' => "ga",
        'ギ' => "gi",
        'グ' => "gu",
        'ゲ' => "ge",
        'ゴ' => "go",
        'サ' => "sa",
        'シ' => "shi",
        'ス' => "su",
        'セ' => "se",
        'ソ' => "so",
        'ザ' => "za",
        'ジ' | 'ヂ' => "ji",
        'ズ' | 'ヅ' => "zu",
        'ゼ' => "ze",
        'ゾ' => "zo",
        'タ' => "ta",
        'チ' => "chi",
        'ツ' => "tsu",
        'テ' => "te",
        'ト' => "to",
        'ダ' => "da",
        'デ' => "de",
        'ド' => "do",
        'ナ' => "na",
        'ニ' => "ni",
        'ヌ' => "nu",
        'ネ' => "ne",
        'ノ' => "no",
        'ハ' => "ha",
        'ヒ' => "hi",
        'フ' => "fu",
        'ヘ' => "he",
        'ホ' => "ho",
        'バ' => "ba",
        'ビ' => "bi",
        'ブ' => "bu",
        'ベ' => "be",
        'ボ' => "bo",
        'パ' => "pa",
        'ピ' => "pi",
        'プ' => "pu",
        'ペ' => "pe",
        'ポ' => "po",
        'マ' => "ma",
        'ミ' => "mi",
        'ム' => "mu",
        'メ' => "me",
        'モ' => "mo",
        'ヤ' | 'ャ' => "ya",
        'ユ' | 'ュ' => "yu",
        'ヨ' | 'ョ' => "yo",
        'ラ' => "ra",
        'リ' => "ri",
        'ル' => "ru",
        'レ' => "re",
        'ロ' => "ro",
        'ワ' | 'ヮ' => "wa",
        'ヲ' => "wo",
        'ン' => "n",
        'ヴ' => "vu",
        _ => return None,
    })
}

/// The Hepburn romaji of a syllable written as a kana and a small one.
fn digraph(kana: char, small: char) -> Option<&'static str> {
    Some(match (kana, small) {
        ('キ', 'ャ') => "kya",
        ('キ', 'ュ') => "kyu",
        ('キ', 'ョ') => "kyo",
        ('ギ', 'ャ') => "gya",
        ('ギ', 'ュ') => "gyu",
        ('ギ', 'ョ') => "gyo",
        ('シ', 'ャ') => "sha",
        ('シ', 'ュ') => "shu",
        ('シ', 'ェ') => "she",
        ('シ', 'ョ') => "sho",
        ('ジ' | 'ヂ', 'ャ') => "ja",
        ('ジ' | 'ヂ', 'ュ') => "ju",
        ('ジ' | 'ヂ', 'ェ') => "je",
        ('ジ' | 'ヂ', 'ョ') => "jo",
        ('チ', 'ャ') => "cha",
        ('チ', 'ュ') => "chu",
        ('チ', 'ェ') => "che",
        ('チ', 'ョ') => "cho",
        ('ニ', 'ャ') => "nya",
        ('ニ', 'ュ') => "nyu",
        ('ニ', 'ョ') => "nyo",
        ('ヒ', 'ャ') => "hya",
        ('ヒ', 'ュ') => "hyu",
        ('ヒ', 'ョ') => "hyo",
        ('ビ', 'ャ') => "bya",
        ('ビ', 'ュ') => "byu",
        ('ビ', 'ョ') => "byo",
        ('ピ', 'ャ') => "pya",
        ('ピ', 'ュ') => "pyu",
        ('ピ', 'ョ') => "pyo",
        ('ミ', 'ャ') => "mya",
        ('ミ', 'ュ') => "myu",
        ('ミ', 'ョ') => "myo",
        ('リ', 'ャ') => "rya",
        ('リ', 'ュ') => "ryu",
        ('リ', 'ョ') => "ryo",
        ('フ', 'ァ') => "fa",
        ('フ', 'ィ') => "fi",
        ('フ', 'ェ') => "fe",
        ('フ', 'ォ') => "fo",
        ('テ', 'ィ') => "ti",
        ('テ', 'ュ') => "tyu",
        ('デ', 'ィ') => "di",
        ('デ', 'ュ') => "dyu",
        ('ト', 'ゥ') => "tu",
        ('ド', 'ゥ') => "du",
        ('ウ', 'ィ') => "wi",
        ('ウ', 'ェ') => "we",
        ('ウ', 'ォ') => "wo",
        ('ツ', 'ァ') => "tsa",
        ('ツ', 'ィ') => "tsi",
        ('ツ', 'ェ') => "tse",
        ('ツ', 'ォ') => "tso",
        ('ヴ', 'ァ') => "va",
        ('ヴ', 'ィ') => "vi",
        ('ヴ', 'ェ') => "ve",
        ('ヴ', 'ォ') => "vo",
        _ => return None,
    })
}

/// Kana as Hepburn romaji, leaving Latin letters and digits as they are. The
/// small ッ and the mark ー add nothing: [`canonical`] folds what they spell.
fn hepburn(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().map(to_katakana).peekable();
    while let Some(c) = chars.next() {
        if let Some(found) = chars.peek().and_then(|small| digraph(c, *small)) {
            out.push_str(found);
            chars.next();
        } else if let Some(found) = syllable(c) {
            out.push_str(found);
        } else if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

fn is_vowel(c: u8) -> bool {
    matches!(c, b'a' | b'i' | b'u' | b'e' | b'o')
}

/// Whether a vowel after `last` only lengthens it, as both toukyou and tookyoo
/// are tokyo. `ei` counts: sensei is said and often typed as sense.
fn lengthens(last: u8, vowel: u8) -> bool {
    matches!(
        (last, vowel),
        (b'o', b'u' | b'o') | (b'u', b'u') | (b'a', b'a') | (b'i', b'i') | (b'e', b'e' | b'i')
    )
}

/// Adds to a spelling, folding doubled consonants and long vowels.
fn push_folded(out: &mut Vec<u8>, c: u8) {
    if let Some(&last) = out.last() {
        let doubled = c == last && !is_vowel(c) && !c.is_ascii_digit();
        if doubled || (is_vowel(c) && lengthens(last, c)) {
            return;
        }
    }
    out.push(c);
}

/// The one spelling of romaji in any system, which every spelling of a reading
/// reaches: Hepburn and Kunrei, doubled consonants, long vowels and ん before
/// b, p and m. Folding it again changes nothing.
pub(crate) fn canonical(romaji: &str) -> String {
    let bytes = romaji.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        let next = bytes.get(ix + 1).copied();
        let after = bytes.get(ix + 2).copied();
        // What this part of the spelling becomes, and how much of it that takes.
        let (spelled, taken): (&[u8], usize) = match (bytes[ix], next, after) {
            (b's', Some(b'h'), Some(b'i')) => (b"si", 3),
            (b's', Some(b'h'), Some(b'a')) => (b"sya", 3),
            (b's', Some(b'h'), Some(b'u')) => (b"syu", 3),
            (b's', Some(b'h'), Some(b'e')) => (b"sye", 3),
            (b's', Some(b'h'), Some(b'o')) => (b"syo", 3),
            (b'c', Some(b'h'), Some(b'i')) => (b"ti", 3),
            (b'c', Some(b'h'), Some(b'a')) => (b"tya", 3),
            (b'c', Some(b'h'), Some(b'u')) => (b"tyu", 3),
            (b'c', Some(b'h'), Some(b'e')) => (b"tye", 3),
            (b'c', Some(b'h'), Some(b'o')) => (b"tyo", 3),
            (b'c', Some(b'y'), _) => (b"ty", 2),
            // The first of cch, as in maccha for matcha.
            (b'c', Some(b'c'), _) => (b"", 1),
            (b't', Some(b's'), Some(b'u')) => (b"tu", 3),
            (b'f', Some(b'u'), _) => (b"hu", 2),
            (b'j', Some(b'y'), _) | (b'd', Some(b'y'), _) => (b"zy", 2),
            (b'j' | b'd', Some(b'i'), _) => (b"zi", 2),
            (b'j', Some(b'a'), _) => (b"zya", 2),
            (b'j', Some(b'u'), _) => (b"zyu", 2),
            (b'j', Some(b'e'), _) => (b"zye", 2),
            (b'j', Some(b'o'), _) => (b"zyo", 2),
            (b'd', Some(b'u'), _) => (b"zu", 2),
            (b'w', Some(b'o'), _) => (b"o", 2),
            (b'm', Some(b'b' | b'p' | b'm'), _) => (b"n", 1),
            _ => (&bytes[ix..=ix], 1),
        };
        for c in spelled {
            push_folded(&mut out, *c);
        }
        ix += taken;
    }
    String::from_utf8(out).expect("every byte is ASCII")
}

/// `canonical` without voicing, for the readings of names that a dictionary
/// tells apart only by it: ヨネツ and ヨネヅ, ゲン and ケン. As long as what it
/// folds, so positions in one hold in the other.
pub(crate) fn unvoiced(canonical: &str) -> String {
    let bytes = canonical.as_bytes();
    let folded = bytes.iter().enumerate().map(|(ix, c)| {
        let before_i_or_u = matches!(bytes.get(ix + 1), Some(b'i' | b'u'));
        match c {
            b't' if before_i_or_u => b's',
            b'g' => b'k',
            b'z' => b's',
            b'd' => b't',
            b'b' | b'p' => b'h',
            c => *c,
        }
    });
    String::from_utf8(folded.collect()).expect("every byte is ASCII")
}

/// Whether a word in [`canonical`] spelling ends where a syllable may.
pub(crate) fn ends_a_syllable(spelled: &str) -> bool {
    spelled
        .as_bytes()
        .last()
        .is_some_and(|c| is_vowel(*c) || *c == b'n')
}

/// What a word in [`canonical`] spelling may become as more of it is typed, when
/// that stops being what it starts with: the `sh` of しん has no place in `sin`.
pub(crate) fn finished_ways(spelled: &str) -> Vec<String> {
    const UNFINISHED: [(&str, &[&str]); 7] = [
        ("sh", &["si", "sy"]),
        ("ch", &["ti", "ty"]),
        ("ts", &["tu"]),
        ("c", &["ti", "ty"]),
        ("j", &["zi", "zy"]),
        ("f", &["hu"]),
        // しんぶん, typed as shimbun.
        ("m", &["nb", "np", "nm"]),
    ];
    UNFINISHED
        .iter()
        .find_map(|(end, ways)| {
            let stem = spelled.strip_suffix(end)?;
            Some(
                ways.iter()
                    .map(|way| canonical(&format!("{stem}{way}")))
                    .collect(),
            )
        })
        .unwrap_or_default()
}

/// The canonical spelling of kana and Latin letters; anything else adds nothing.
pub(crate) fn spelling(text: &str) -> String {
    canonical(&hepburn(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_system_spells_one_way() {
        for typed in ["shinde", "sinde"] {
            assert_eq!(canonical(typed), "sinde");
        }
        for typed in ["chikyuu", "tikyu", "chikyu"] {
            assert_eq!(canonical(typed), "tikyu");
        }
        for typed in ["tsuki", "tuki"] {
            assert_eq!(canonical(typed), "tuki");
        }
        for typed in ["fuji", "huzi", "fuzi", "huji"] {
            assert_eq!(canonical(typed), "huzi");
        }
        for typed in ["jyunjou", "junjo", "zyunzyou"] {
            assert_eq!(canonical(typed), "zyunzyo");
        }
        for typed in ["shashin", "syasin"] {
            assert_eq!(canonical(typed), "syasin");
        }
    }

    #[test]
    fn long_vowels_and_doubled_consonants_fold() {
        for typed in ["toukyou", "tokyo", "tookyoo"] {
            assert_eq!(canonical(typed), "tokyo");
        }
        for typed in ["gakkou", "gakou", "gako"] {
            assert_eq!(canonical(typed), "gako");
        }
        for typed in ["matcha", "maccha", "matya"] {
            assert_eq!(canonical(typed), "matya");
        }
        assert_eq!(canonical("sensei"), canonical("sense"));
    }

    #[test]
    fn n_before_b_p_and_m_may_be_written_m() {
        assert_eq!(canonical("shimbashi"), canonical("shinbashi"));
        assert_eq!(canonical("sampo"), canonical("sanpo"));
        assert_eq!(canonical("konnichiwa"), canonical("konichiwa"));
    }

    #[test]
    fn folding_again_changes_nothing() {
        for typed in [
            "toukyou",
            "shinjuku",
            "chocho",
            "maccha",
            "shimbashi",
            "wooo",
            "sensei",
        ] {
            let once = canonical(typed);
            assert_eq!(canonical(&once), once);
        }
    }

    #[test]
    fn kana_spell_as_their_romaji_does() {
        assert_eq!(spelling("トウキョウ"), canonical("toukyou"));
        assert_eq!(spelling("トーキョー"), canonical("toukyou"));
        assert_eq!(spelling("がっこう"), canonical("gakkou"));
        assert_eq!(spelling("ジュンジョウ"), canonical("junjou"));
        assert_eq!(spelling("コーヒー"), canonical("koohii"));
        assert_eq!(spelling("ヂ"), spelling("ジ"));
        assert_eq!(spelling("ヅ"), spelling("ズ"));
        assert_eq!(spelling("ヲ"), spelling("オ"));
    }

    #[test]
    fn latin_letters_and_digits_stay_and_the_rest_goes() {
        assert_eq!(spelling("Mステ2"), "msute2");
        assert_eq!(spelling("夜"), "");
        // Digits may double, unlike consonants.
        assert_eq!(spelling("1100"), "1100");
    }

    #[test]
    fn unvoiced_spellings_meet_across_voicing_and_keep_their_length() {
        let meets = |a: &str, b: &str| assert_eq!(unvoiced(&spelling(a)), unvoiced(&spelling(b)));
        meets("ヨネヅ", "ヨネツ");
        meets("ゲンシ", "ケンシ");
        meets("ジ", "シ");
        meets("チ", "ジ");
        meets("バ", "ハ");
        meets("パ", "ハ");
        meets("ダ", "タ");
        let spelled = spelling("ヨネヅケンシ");
        assert_eq!(unvoiced(&spelled).len(), spelled.len());
    }
}
