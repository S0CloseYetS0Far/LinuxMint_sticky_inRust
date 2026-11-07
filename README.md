# Sticky (Rust port)
![rust](https://github.com/S0CloseYetS0Far/LinuxMint_sticky_inRust/actions/workflows/rust.yml/badge.svg)

Sticky is a note-taking app for the Linux desktop that simulates traditional "sticky note" style stationery on your desktop. Some of its features include basic text formatting (bold, italics, monospaced, etc.), spell-checking, a tray icon for controlling note visibility, color notes, manual and automatic backups, and a manager to organize your notes into groups.

This is a port of [linuxmint/sticky](https://github.com/linuxmint/sticky) from Python to Rust. It uses GTK3 through [gtk-rs](https://gtk-rs.org), plus small hand-written bindings for libxapp and gspell (`src/ffi.rs`). It keeps the same settings schema (`org.x.sticky`), notes file (`~/.config/sticky/notes.json`), D-Bus interface, translations and data files, so it can replace the Python version in place.

![img](https://linuxmint.com/pictures/screenshots/uma/sticky.png)

## Python vs Rust

Both versions were run on the same machine with the same settings and the same `notes.json`, which has 5 notes with a checklist, bullets and a link. Each was run 11 times and the first run was not counted. The tables show the median of the other 10.

| | Python ([linuxmint/sticky](https://github.com/linuxmint/sticky)) | Rust (this repo) | Difference |
|---|---|---|---|
| **Startup time** (launch → first note on screen) | 258 ms | 98 ms | **2.6× faster** |
| **RAM, RSS** (all memory the process has mapped in) | 106.5 MiB | 79.1 MiB | −27.4 MiB (−26 %) |
| **RAM, PSS** (shared libraries split between the processes using them) | 89.5 MiB | 67.4 MiB | −22.1 MiB (−25 %) |
| **RAM, private** (memory only this process uses) | 81.8 MiB | 62.3 MiB | −19.5 MiB (−24 %) |
| **Program size on disk** | 120 KiB of `.py` files, plus the `__pycache__` bytecode written on first run | 2.3 MiB, one stripped binary | |
| **Extra packages needed to run it** | about 26.7 MiB (see below) | none | −26.7 MiB |
| **Lines of code** | 3,301 (5 files) | 5,442 (14 files, including tests) | |

The packages that only the Python version needs, with their installed sizes from `dpkg`:

| Package | Size |
|---|---|
| Python 3.14 interpreter and standard library (`python3`, `python3-minimal`, `python3.14`, `python3.14-minimal`, `libpython3.14-minimal`, `libpython3.14-stdlib`, `libpython3-stdlib`) | 23.7 MiB |
| PyGObject and its typelibs (`python3-gi`, `libgirepository-2.0-0`, `gir1.2-glib-2.0`, `gir1.2-gtk-3.0`, `gir1.2-xapp-1.0`, `gir1.2-gspell-1`) | 2.8 MiB |
| `python3-xapp` | 0.3 MiB |

Both versions also need the same shared libraries (GTK 3, libxapp, gspell) and the same data files (icons, CSS, the manager UI and translations, about 76 KiB without the translations). Those are left out of the comparison.

Python is usually installed on Linux Mint anyway, because other system tools use it. So on Mint the saving is mostly in startup time and RAM, and on a minimal system it is also the 26.7 MiB of extra packages.

<details>
<summary>How it was measured</summary>

- **Machine:** Intel Core i5-10600K, 16 GB RAM, Ubuntu 26.04 in WSL2 (WSLg X server), GTK 3.24, `GDK_BACKEND=x11`.
- **Builds:** Python from `linuxmint/sticky` master, run with `python3 sticky.py`. Rust from this repo built with `cargo build --release` (LTO) and stripped.
- **Each run:** a fresh D-Bus session (`dbus-run-session`), a fresh config folder and warm disk caches. The settings were `first-run=false`, `show-manager=false` and `automatic-backups=false`.
- **Startup:** the time from starting the process until the first sticky window is reported as `IsViewable` by `xwininfo`, which was polled in a loop.
- **RAM:** read from `/proc/<pid>/smaps_rollup` 3 seconds after the first window appeared.
- **Accuracy:** expect different absolute numbers on a real Mint desktop. The gap between the two versions comes mostly from the Python interpreter, PyGObject and importing the modules, and that part does not depend on the desktop.

</details>

## Project structure

```
LinuxMint_sticky_inRust/
├── Cargo.toml, Cargo.lock      crate manifest (gtk-rs 0.18, serde_json, regex, gettext-rs, …)
├── build.rs                    links libxapp and gspell through pkg-config
├── src/
│   ├── main.rs                 entry point: sets up translations, runs the app
│   ├── app.rs                  the GtkApplication: command line, D-Bus, tray icon, notes, menus
│   ├── note.rs                 one note window: title, color, text view, shortcuts, menus
│   ├── note_buffer.rs          the note text: formatting tags, checklists, bullets, links, undo/redo
│   ├── check_box.rs            the check box used in checklists
│   ├── manager.rs              the notes manager: groups, previews, search, drag and drop
│   ├── file_handler.rs         loading and saving notes.json, backups, import/export, groups
│   ├── notes.rs                the notes.json format (keeps key order and unknown keys)
│   ├── settings_window.rs      preferences and keyboard shortcuts windows
│   ├── widgets.rs              shared widgets: hover box, confirm and error dialogs
│   ├── util.rs                 URL detection, note markup parser, Gnote import
│   ├── signal.rs               a small signal type used between the modules
│   ├── i18n.rs                 gettext helpers
│   └── ffi.rs                  hand-written bindings for XApp (StatusIcon, StyleManager) and gspell
├── usr/
│   ├── bin/sticky              launcher: runs /usr/lib/sticky/sticky
│   └── share/
│       ├── glib-2.0/schemas/   org.x.sticky settings schema (same as the Python version)
│       ├── icons/hicolor/      app and toolbar icons
│       └── sticky/             manager.ui, sticky.css, check box images
├── data/                       .desktop file and D-Bus service templates
├── etc/xdg/autostart/          autostart entry
├── po/, sticky.pot, makepot    translations
├── meson.build, meson/         Meson build: runs cargo and installs everything
├── debian/                     Debian packaging (builds with cargo)
└── .github/workflows/rust.yml  CI: build, tests under Xvfb, clippy
```

How the Rust files map to the Python ones:

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
| `src/signal.rs`, `src/i18n.rs`, `src/ffi.rs` | (PyGObject did this) | signals, gettext, C bindings |

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
    - Takes 1 additional argument containing the text of the new note
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

## Contributing

Contributions are welcome. A pull request is only reviewed if it follows these rules.

### 1. Say whether AI was used, and which model

Fill in the **AI disclosure** section of the pull request template (it appears when you open a PR):

- **No AI used:** write `None`.
- **AI used:** give the tool and the exact model with its version, for example `Claude Code, Claude Opus 5.5` or `GitHub Copilot, GPT-5`. Say which parts it wrote: all of the change, some functions, only tests, only the description, and so on.

"An AI" or "ChatGPT" is not enough. Give the model name.

### 2. Say whether you understand the code

The template asks you to pick one:

- **Yes:** you understand every line you are submitting. You can explain why it works and answer review questions about it without asking the AI again.
- **Partly:** list the parts you do not fully understand, so reviewers can look at them closely.

Pull requests that leave this blank are closed. "Partly" is allowed, but code that nobody understands, including the person submitting it, is not merged. That matters most for `unsafe` blocks and anything in `src/ffi.rs`.

### 3. Sign off every commit and take responsibility for the code

Every commit must end with a `Signed-off-by` line with your real name and email:

```
Signed-off-by: Your Name <you@example.com>
```

`git commit -s` adds it for you. If you forgot, `git commit --amend -s` fixes the last commit and `git rebase --signoff master` fixes all of them.

Signing off means you state that:

1. You wrote the change, or you have the right to submit it under the project's license (GPLv2), as described in the [Developer Certificate of Origin](https://developercertificate.org/).
2. **You take responsibility for the code**, including any part an AI wrote. It is your contribution, and "the AI did it" is not an answer to a bug, a licence problem or a review question.
3. You have built it and run it (`cargo build`, `cargo test -- --test-threads=1`, `cargo clippy -- -D warnings`) and checked that it does what the PR says.

The PR template asks you to confirm these points. A `Co-Authored-By` line for an AI tool is fine in addition to your sign-off, but it never replaces it.

### Other rules

- One fix or feature per pull request, and one commit per logical change.
- Reference the issue it fixes (`Fixes #12`).
- Keep the settings schema, the `notes.json` format and the D-Bus interface compatible with the Python version, unless the PR is about changing them on purpose.

## License
- Code: GPLv2
