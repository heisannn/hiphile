/// Gives the process a `LANG` from the user's locale when it has no locale in its environment, as
/// an app opened from Finder does not. AntiWebUI reads `LC_ALL`, `LC_CTYPE` and then `LANG` for the
/// language it chooses glyphs by, such as Japanese kanji over Chinese hanzi.
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

/// The first language of a list as `defaults read` prints one, such as `en-JP` of
/// `("en-JP", "ja-JP")`. A name without a hyphen comes without quotes.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn first_language(list: &str) -> Option<&str> {
    list.trim()
        .strip_prefix('(')?
        .strip_suffix(')')?
        .split(',')
        .map(|language| language.trim().trim_matches('"'))
        .find(|language| !language.is_empty())
}

#[cfg(test)]
mod tests;
