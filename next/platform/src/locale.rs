/// Gives the process a `LANG` from the user's locale when it has no locale in its environment, as
/// an app opened from Finder does not. AntiWebUI reads `LC_ALL`, `LC_CTYPE` and then `LANG` for the
/// language it chooses glyphs by, such as Japanese kanji over Chinese hanzi, so a locale such as
/// `en_JP` gives way to the first of the user's languages that has glyphs of its own for Han.
///
/// Call first in `main`. Once another thread runs it leaves the environment alone, since setting it
/// could race with that thread reading it. Elsewhere than on macOS it does nothing.
pub fn set_lang() {
    #[cfg(target_os = "macos")]
    if !["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
        && let Some(lang) = crate::mac::locale::user_locale()
            .as_deref()
            .and_then(lang_for)
    {
        crate::mac::locale::set_lang(&lang);
    }
}

/// What `LANG` says for the locale named `identifier`, such as `ja_JP.UTF-8` for `ja_JP`. Keywords
/// after `@` are left out, and a script stays: `zh-Hans_CN.UTF-8`, which AntiWebUI reads as
/// `zh-Hans-CN`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn lang_for(identifier: &str) -> Option<String> {
    let name = identifier.split('@').next().unwrap_or_default();
    let starts_with_language = name.starts_with(|c: char| c.is_ascii_alphabetic());
    let named = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    (starts_with_language && named).then(|| format!("{name}.UTF-8"))
}

/// The locale to choose glyphs by: `locale`, the user's, unless its language has no glyphs of its
/// own for Han and one of `languages` does, or else the first of them. `languages` is the list as
/// `defaults read` prints it, read only if needed.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn locale_for(
    locale: Option<String>,
    languages: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if locale.as_deref().is_some_and(has_own_han) {
        return locale;
    }
    let list = languages().unwrap_or_default();
    let listed = languages_in(&list);
    let chosen = listed.iter().find(|language| has_own_han(language));
    match (chosen, locale) {
        (Some(language), _) => Some(locale_of(language)),
        (None, Some(locale)) => Some(locale),
        (None, None) => listed.first().map(|language| locale_of(language)),
    }
}

/// Whether the language of `identifier` has glyphs of its own for Han: Japanese, Chinese or
/// Korean.
fn has_own_han(identifier: &str) -> bool {
    let language = identifier.split(['-', '_', '@']).next().unwrap_or_default();
    matches!(language, "ja" | "zh" | "ko")
}

/// The languages of a list as `defaults read` prints one, such as `en-JP` and `ja-JP` of
/// `("en-JP", "ja-JP")`. A name without a hyphen comes without quotes.
fn languages_in(list: &str) -> Vec<&str> {
    let Some(inside) = list
        .trim()
        .strip_prefix('(')
        .and_then(|list| list.strip_suffix(')'))
    else {
        return Vec::new();
    };
    inside
        .split(',')
        .map(|language| language.trim().trim_matches('"'))
        .filter(|language| !language.is_empty())
        .collect()
}

/// The locale a language names, as macOS writes one: `zh-Hans_CN` for `zh-Hans-CN`.
fn locale_of(language: &str) -> String {
    match language.rsplit_once('-') {
        Some((before, region))
            if region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic())
                || region.len() == 3 && region.chars().all(|c| c.is_ascii_digit()) =>
        {
            format!("{before}_{region}")
        }
        _ => language.to_owned(),
    }
}

#[cfg(test)]
mod tests;
