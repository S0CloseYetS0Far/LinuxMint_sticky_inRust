//! Translations, using the same "sticky" gettext domain and .po files as the
//! Python version.

pub const DOMAIN: &str = "sticky";
pub const LOCALE_DIR: &str = "/usr/share/locale";

pub fn init() {
    use gettextrs::{bind_textdomain_codeset, bindtextdomain, setlocale, textdomain, LocaleCategory};
    setlocale(LocaleCategory::LcAll, "");
    let _ = bindtextdomain(DOMAIN, LOCALE_DIR);
    let _ = bind_textdomain_codeset(DOMAIN, "UTF-8");
    let _ = textdomain(DOMAIN);
}

#[cfg(not(test))]
pub fn gettext(msg: &str) -> String {
    gettextrs::gettext(msg)
}

#[cfg(test)]
pub fn gettext(msg: &str) -> String {
    msg.to_string()
}

/// Translate a message containing a single `%s` and fill it in.
pub fn gettext_fmt(msg: &str, arg: &str) -> String {
    gettext(msg).replacen("%s", arg, 1)
}

/// Marks a string for translation without translating it (for constant
/// tables); pass the value to `gettext` where it is shown.
pub const fn n_(msg: &str) -> &str {
    msg
}
