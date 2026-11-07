// Sticky uses two C libraries that have no Rust bindings: libxapp (tray icon,
// preferences window, font style manager) and gspell (inline spell checking).
// Their few functions are declared by hand in src/ffi.rs; this links them.
fn main() {
    for lib in ["xapp", "gspell-1"] {
        if let Err(e) = pkg_config::Config::new().probe(lib) {
            panic!("could not find {lib} with pkg-config: {e}");
        }
    }
}
