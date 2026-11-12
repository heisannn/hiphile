use super::*;

#[test]
fn a_locale_becomes_its_lang_in_utf_8() {
    assert_eq!(lang_for("ja_JP").as_deref(), Some("ja_JP.UTF-8"));
    assert_eq!(lang_for("es_419").as_deref(), Some("es_419.UTF-8"));
    assert_eq!(lang_for("en").as_deref(), Some("en.UTF-8"));
}

#[test]
fn keywords_are_left_out_and_a_script_stays() {
    assert_eq!(
        lang_for("ja_JP@calendar=japanese").as_deref(),
        Some("ja_JP.UTF-8")
    );
    assert_eq!(
        lang_for("zh-Hans_CN@rg=twzzzz").as_deref(),
        Some("zh-Hans_CN.UTF-8")
    );
}

#[test]
fn what_names_no_locale_gives_none() {
    assert_eq!(lang_for(""), None);
    assert_eq!(lang_for("@calendar=japanese"), None);
    assert_eq!(lang_for("_JP"), None);
    assert_eq!(lang_for("ja JP"), None);
    assert_eq!(lang_for("ja_JP.UTF-8"), None);
}

/// A list of languages as `defaults read` prints it.
fn printed(languages: &[&str]) -> Option<String> {
    let lines: Vec<String> = languages
        .iter()
        .map(|language| format!("    \"{language}\""))
        .collect();
    Some(format!("(\n{}\n)\n", lines.join(",\n")))
}

fn unread() -> Option<String> {
    panic!("the languages were read")
}

#[test]
fn the_languages_are_read_from_what_defaults_prints() {
    assert_eq!(
        languages_in("(\n    \"en-JP\",\n    \"ja-JP\"\n)\n"),
        ["en-JP", "ja-JP"]
    );
    assert_eq!(languages_in("(\n    ja\n)\n"), ["ja"]);
    assert!(languages_in("(\n)\n").is_empty());
    assert!(languages_in("").is_empty());
}

#[test]
fn a_locale_whose_language_has_its_own_han_is_kept() {
    for locale in ["ja_JP", "zh-Hant_TW", "ko_KR", "ja_JP@calendar=japanese"] {
        assert_eq!(
            locale_for(Some(locale.to_owned()), unread).as_deref(),
            Some(locale)
        );
    }
}

#[test]
fn other_locales_give_way_to_the_first_language_with_its_own_han() {
    let after_english = |locale: &str, languages: &[&str]| {
        locale_for(Some(locale.to_owned()), || printed(languages))
    };
    assert_eq!(
        after_english("en_JP", &["en-JP", "ja-JP"]).as_deref(),
        Some("ja_JP")
    );
    assert_eq!(
        after_english("en_US", &["en-US", "fr-FR", "zh-Hans-CN", "ja-JP"]).as_deref(),
        Some("zh-Hans_CN")
    );
    assert_eq!(
        after_english("fr_FR", &["fr-FR", "ko"]).as_deref(),
        Some("ko")
    );
    assert_eq!(
        after_english("en_US", &["en-US", "fr-FR"]).as_deref(),
        Some("en_US")
    );
    assert_eq!(
        locale_for(Some("en_US".to_owned()), || None).as_deref(),
        Some("en_US")
    );
}

#[test]
fn without_a_locale_a_language_names_one() {
    assert_eq!(
        locale_for(None, || printed(&["en-JP", "ja-JP"])).as_deref(),
        Some("ja_JP")
    );
    assert_eq!(
        locale_for(None, || printed(&["es-419", "fr-FR"])).as_deref(),
        Some("es_419")
    );
    assert_eq!(
        locale_for(None, || printed(&["zh-Hans"])).as_deref(),
        Some("zh-Hans")
    );
    assert_eq!(locale_for(None, || printed(&[])), None);
    assert_eq!(locale_for(None, || None), None);
}

#[cfg(target_os = "macos")]
#[test]
fn a_test_is_not_alone_in_its_process() {
    assert!(!crate::mac::locale::is_alone());
}
