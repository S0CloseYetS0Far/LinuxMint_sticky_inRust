# Sticky (Rust port)
![rust](https://github.com/S0CloseYetS0Far/LinuxMint_sticky_inRust/actions/workflows/rust.yml/badge.svg)

Sticky is a note-taking app for the Linux desktop that simulates traditional "sticky note" style stationery on your desktop. Some of its features include basic text formatting (bold, italics, monospaced, etc.), spell-checking, a tray icon for controlling note visibility, color notes, manual and automatic backups, and a manager to organize your notes into groups.

This is a port of [linuxmint/sticky](https://github.com/linuxmint/sticky) from Python to Rust. It uses GTK3 through [gtk-rs](https://gtk-rs.org), plus small hand-written bindings for libxapp and gspell (`src/ffi.rs`). It keeps the same settings schema (`org.x.sticky`), notes file (`~/.config/sticky/notes.json`), D-Bus interface, translations and data files, so it can replace the Python version in place.

![img](https://linuxmint.com/pictures/screenshots/uma/sticky.png)

## Source layout

| File | Python original | What it does |
|------|-----------------|--------------|
| `src/app.rs` | `sticky.py` (`Application`) | command line, D-Bus, tray icon, note placement |
| `src/note.rs` | `sticky.py` (`Note`) | a note window |
| `src/settings_window.rs` | `sticky.py` (`SettingsWindow`, `ShortcutsWindow`) | preferences and shortcuts |
| `src/note_buffer.rs`, `src/check_box.rs` | `note_buffer.py` | formatting, checklists, links, undo/redo |
| `src/manager.rs` | `manager.py` | the notes manager window |
| `src/file_handler.rs`, `src/notes.rs` | `common.py` (`FileHandler`) | loading, saving, backups |
| `src/widgets.rs` | `common.py` (`HoverBox`, `confirm`) | shared widgets |
| `src/util.rs` | `util.py` | URL detection, markup parsing, Gnote import |

## How to build and install

Build dependencies (Mint/Debian/Ubuntu): `cargo rustc pkg-config libgtk-3-dev libgspell-1-dev libxapp-dev meson`.

```
git clone https://github.com/S0CloseYetS0Far/LinuxMint_sticky_inRust.git
cd LinuxMint_sticky_inRust
cargo build --release
cargo test -- --test-threads=1    # the GTK tests need a display
```

### Mint, Debian, Ubuntu and derivatives:
```
dpkg-buildpackage --no-sign
cd ..
sudo dpkg -i sticky*.deb
```
### Build and install using Meson
Meson builds the Rust program with cargo and installs it as `/usr/lib/sticky/sticky`, together with the data files:
```
meson setup builddir --prefix=/usr
meson compile -C builddir
sudo meson install -C builddir
```
### Running from the source tree
`STICKY_DATA_DIR` points the program at the UI and CSS files:
```
glib-compile-schemas usr/share/glib-2.0/schemas
GSETTINGS_SCHEMA_DIR=usr/share/glib-2.0/schemas STICKY_DATA_DIR=usr/share/sticky GDK_BACKEND=x11 cargo run
```

#### runtime deps
- libgtk-3-0 (>= 3.22)
- libxapp1 (>= 2.2.0)
- libgspell-1-3

## Controlling Sticky via DBUS
Sticky offers the following dbus methods and signals:
- 'ShowNotes' (method): toggles visibility and focus of the notes on the screen
- 'NewNote' (method): creates a new note containing the provided text
    - Takes 1 addtional argument containing the text of the new note
- 'NewNoteBlank' (method): creates a new empty note
- 'NotesChanged' (signal): indicates that the notes have changed in some way and the changes have been saved

### Command Line
You can use dbus-send to call dbus methods (and dbus-monitor for signals) at a command prompt, in a script, etc.
```
dbus-send --type=method_call --dest="org.x.sticky" /org/x/sticky org.x.sticky.ShowNotes
dbus-send --type=method_call --dest="org.x.sticky" /org/x/sticky org.x.sticky.NewNoteBlank
dbus-send --type=method_call --dest="org.x.sticky" /org/x/sticky org.x.sticky.NewNote string:'New note text'
dbus-monitor "type='signal',interface='org.x.sticky',member=NotesChanged"
```

## Translations
Please use Launchpad to translate Sticky Notes (upstream): https://translations.launchpad.net/linuxmint/latest/.

The PO files in this project are imported from there.

## License
- Code: GPLv2
