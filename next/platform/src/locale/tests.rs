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

#[test]
fn the_first_language_is_read_from_what_defaults_prints() {
    assert_eq!(
        first_language("(\n    \"en-JP\",\n    \"ja-JP\"\n)\n"),
        Some("en-JP")
    );
    assert_eq!(first_language("(\n    ja\n)\n"), Some("ja"));
    assert_eq!(first_language("(\n)\n"), None);
    assert_eq!(first_language(""), None);
}

#[cfg(target_os = "macos")]
#[test]
fn a_test_is_not_alone_in_its_process() {
    assert!(!crate::mac::locale::is_alone());
}
