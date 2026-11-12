//! `set_lang` at the start of a process, as `main` calls it. Each case runs in a process of its own,
//! which is this program started again with the environment of the case.

use std::process::Command;

/// Set in the processes started again, which call `set_lang` and print `LANG`.
const CASE: &str = "NEXT_PLATFORM_LANG_CASE";

fn main() {
    if std::env::var_os(CASE).is_some() {
        next_platform::locale::set_lang();
        print!("{}", std::env::var("LANG").unwrap_or_default());
        return;
    }
    without_a_locale_it_gets_one_from_the_user();
    a_lang_given_is_kept();
    a_locale_in_lc_ctype_leaves_lang_unset();
    println!("lang: 3 passed");
}

fn without_a_locale_it_gets_one_from_the_user() {
    let lang = lang_after(|_| {});
    if cfg!(target_os = "macos") {
        assert!(lang.ends_with(".UTF-8"), "LANG was {lang:?}");
    } else {
        assert_eq!(lang, "");
    }
}

fn a_lang_given_is_kept() {
    let lang = lang_after(|case| {
        case.env("LANG", "fr_FR.UTF-8");
    });
    assert_eq!(lang, "fr_FR.UTF-8");
}

fn a_locale_in_lc_ctype_leaves_lang_unset() {
    let lang = lang_after(|case| {
        case.env("LC_CTYPE", "fr_FR.UTF-8");
    });
    assert_eq!(lang, "");
}

/// What `LANG` is after `set_lang` in a process that starts with no locale in its environment, as
/// one opened from Finder does, and then what `set_up` adds.
fn lang_after(set_up: impl FnOnce(&mut Command)) -> String {
    let mut case = Command::new(std::env::current_exe().expect("the test's own path"));
    case.env(CASE, "1")
        .env_remove("LC_ALL")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    set_up(&mut case);
    let ran = case.output().expect("the test started again");
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    String::from_utf8(ran.stdout).expect("LANG in UTF-8")
}
