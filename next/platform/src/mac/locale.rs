use std::ffi::c_void;
use std::mem::{MaybeUninit, size_of};
use std::process::Command;

/// The name of the locale to choose glyphs by, such as `ja_JP`, from the user's locale and their
/// languages as `crate::locale::locale_for` says. `defaults` reads them in a process of its own:
/// read here, through `NSLocale`, CFPreferences would start threads to talk to `cfprefsd`, and
/// setting `LANG` would no longer be safe.
pub(crate) fn user_locale() -> Option<String> {
    let global = |key: &str| {
        let read = Command::new("/usr/bin/defaults")
            .args(["read", "-g", key])
            .output()
            .ok()?;
        read.status
            .success()
            .then(|| String::from_utf8(read.stdout).ok())
            .flatten()
    };
    let locale = global("AppleLocale").map(|locale| locale.trim().to_owned());
    crate::locale::locale_for(locale, || global("AppleLanguages"))
}

/// Sets `LANG` to `lang` if this is the process's only thread. Whether it did.
pub(crate) fn set_lang(lang: &str) -> bool {
    if !is_alone() {
        return false;
    }
    // SAFETY: no other thread is running to read the environment meanwhile, and only this one could
    // start one
    unsafe { std::env::set_var("LANG", lang) };
    true
}

/// Whether this is the process's only thread.
pub(crate) fn is_alone() -> bool {
    let mut info = MaybeUninit::<libc::proc_taskinfo>::uninit();
    let size = size_of::<libc::proc_taskinfo>() as libc::c_int;
    // SAFETY: the buffer is as large as the size given, which is what this flavor fills
    let filled = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast::<c_void>(),
            size,
        )
    };
    if filled != size {
        return false;
    }
    // SAFETY: filled in whole, as `proc_pidinfo` said
    let info = unsafe { info.assume_init() };
    info.pti_threadnum == 1
}
