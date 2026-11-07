//! Loading, saving and backing up `~/.config/sticky/notes.json`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use gtk::prelude::*;

use crate::i18n::{gettext, gettext_fmt};
use crate::notes::{parse_notes, serialize_notes, NoteInfo, NotesLists};
use crate::signal::Signal;
use crate::widgets::{confirm, show_error};

const SAVE_DELAY: u32 = 3;

pub fn config_dir() -> PathBuf {
    glib::user_config_dir().join("sticky")
}

pub fn config_path() -> PathBuf {
    config_dir().join("notes.json")
}

/// `backup-<unix time>.json`, at least ten digits.
pub fn is_backup_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(digits) = lower.strip_prefix("backup-").and_then(|s| s.strip_suffix(".json")) else {
        return false;
    };
    digits.len() >= 10 && digits.bytes().all(|b| b.is_ascii_digit())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Sorted backup file names found in `dir`.
pub fn list_backups(dir: &Path) -> Vec<String> {
    let mut backups: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| is_backup_file_name(n))
                .collect()
        })
        .unwrap_or_default();
    backups.sort();
    backups
}

pub struct FileHandler {
    settings: gio::Settings,
    window: gtk::Window,
    notes_lists: RefCell<NotesLists>,
    save_timer_id: RefCell<Option<glib::SourceId>>,
    backup_timer_id: RefCell<Option<glib::SourceId>>,
    weak_self: RefCell<Weak<FileHandler>>,

    pub group_changed: Signal<String>,
    pub group_name_changed: Signal<(String, String)>,
    pub lists_changed: Signal<()>,
    pub saved: Signal<()>,
}

impl FileHandler {
    pub fn new(settings: &gio::Settings, window: &gtk::Window) -> Rc<Self> {
        let handler = Rc::new(FileHandler {
            settings: settings.clone(),
            window: window.clone(),
            notes_lists: RefCell::default(),
            save_timer_id: RefCell::default(),
            backup_timer_id: RefCell::default(),
            weak_self: RefCell::default(),
            group_changed: Signal::default(),
            group_name_changed: Signal::default(),
            lists_changed: Signal::default(),
            saved: Signal::default(),
        });
        *handler.weak_self.borrow_mut() = Rc::downgrade(&handler);

        if config_path().exists() {
            handler.load_notes();
        }

        for key in ["automatic-backups", "backup-interval"] {
            let weak = Rc::downgrade(&handler);
            settings.connect_changed(Some(key), move |_, _| {
                if let Some(h) = weak.upgrade() {
                    h.check_backup();
                }
            });
        }
        handler.check_backup();

        handler
    }

    fn weak(&self) -> Weak<FileHandler> {
        self.weak_self.borrow().clone()
    }

    pub fn load_notes(&self) {
        match std::fs::read_to_string(config_path()).map_err(|e| e.to_string()).and_then(|d| parse_notes(&d).map_err(|e| e.to_string())) {
            Ok(lists) => *self.notes_lists.borrow_mut() = lists,
            Err(e) => eprintln!("sticky: could not read {}: {e}", config_path().display()),
        }
    }

    pub fn get_note_list(&self, group_name: &str) -> Vec<NoteInfo> {
        self.notes_lists.borrow().get(group_name).cloned().unwrap_or_default()
    }

    pub fn get_note_group_names(&self) -> Vec<String> {
        self.notes_lists.borrow().keys().cloned().collect()
    }

    pub fn has_group(&self, group_name: &str) -> bool {
        self.notes_lists.borrow().contains_key(group_name)
    }

    pub fn update_note_list(&self, notes: Vec<NoteInfo>, group_name: &str) {
        self.notes_lists.borrow_mut().insert(group_name.to_string(), notes);
        self.queue_save();
        self.group_changed.emit(&group_name.to_string());
    }

    fn queue_save(&self) {
        if let Some(id) = self.save_timer_id.borrow_mut().take() {
            id.remove();
        }
        let weak = self.weak();
        let id = glib::timeout_add_seconds_local(SAVE_DELAY, move || {
            if let Some(h) = weak.upgrade() {
                h.save_timer_id.borrow_mut().take();
                h.save_note_list();
            }
            glib::ControlFlow::Break
        });
        *self.save_timer_id.borrow_mut() = Some(id);
    }

    fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        let data = serialize_notes(&self.notes_lists.borrow());
        std::fs::write(path, data)
    }

    fn save_note_list(&self) {
        if let Err(e) = std::fs::create_dir_all(config_dir()).and_then(|_| self.save_to_file(&config_path())) {
            eprintln!("sticky: could not save notes: {e}");
        }
        self.saved.emit(&());
    }

    pub fn check_backup(&self) {
        if let Some(id) = self.backup_timer_id.borrow_mut().take() {
            id.remove();
        }

        if !self.settings.boolean("automatic-backups") {
            return;
        }

        let now = now();
        let mut last_backup = self.settings.uint("latest-backup") as u64;
        let interval = self.settings.uint("backup-interval") as u64;

        if last_backup == 0 {
            // unless it was reset, this means the application was just started for the first time, so there's no
            // point in running a backup yet
            let _ = self.settings.set_uint("latest-backup", now as u32);
            last_backup = now;
        }

        let next_backup = last_backup + interval * 3600;

        if next_backup < now {
            self.save_backup();
        } else {
            let weak = self.weak();
            let id = glib::timeout_add_seconds_local((next_backup - now) as u32, move || {
                if let Some(h) = weak.upgrade() {
                    h.backup_timer_id.borrow_mut().take();
                    h.save_backup();
                }
                glib::ControlFlow::Break
            });
            *self.backup_timer_id.borrow_mut() = Some(id);
        }
    }

    pub fn save_backup(&self) {
        if let Some(id) = self.backup_timer_id.borrow_mut().take() {
            id.remove();
        }

        let dir = config_dir();
        let _ = std::fs::create_dir_all(&dir);

        let timestamp = now();
        if let Err(e) = self.save_to_file(&dir.join(format!("backup-{timestamp}.json"))) {
            eprintln!("sticky: could not write backup: {e}");
        }

        let _ = self.settings.set_uint("latest-backup", timestamp as u32);

        // remove old backups (if applicable)
        let keep = self.settings.uint("old-backups-max") as usize;
        if keep > 0 {
            let backups = list_backups(&dir);
            let remove = backups.len().saturating_sub(keep);
            for file in &backups[..remove] {
                let _ = std::fs::remove_file(dir.join(file));
            }
        }

        self.check_backup();
    }

    pub fn delete_all_backups(&self) {
        let dir = config_dir();
        for file in list_backups(&dir) {
            let _ = std::fs::remove_file(dir.join(file));
        }
    }

    fn add_filters(dialog: &gtk::FileChooserDialog) {
        let json = gtk::FileFilter::new();
        json.set_name(Some("JSON"));
        json.add_mime_type("application/json");
        dialog.add_filter(json);

        let text = gtk::FileFilter::new();
        text.set_name(Some(&gettext("Plain Text")));
        text.add_mime_type("text/plain");
        dialog.add_filter(text);
    }

    pub fn export_notes(&self, window: &gtk::Window) {
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(&gettext("Export...")),
            Some(window),
            gtk::FileChooserAction::Save,
            &[(&gettext("Cancel"), gtk::ResponseType::Cancel), (&gettext("Save"), gtk::ResponseType::Ok)],
        );
        dialog.set_current_folder(glib::home_dir());
        dialog.set_current_name("notes.json");
        dialog.set_do_overwrite_confirmation(true);
        Self::add_filters(&dialog);

        if dialog.run() == gtk::ResponseType::Ok {
            if let Some(file) = dialog.filename() {
                if let Err(e) = self.save_to_file(&file) {
                    eprintln!("sticky: export failed: {e}");
                }
            }
        }
        unsafe { dialog.destroy() };
    }

    pub fn import_notes(&self, window: &gtk::Window) {
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(&gettext("Import...")),
            Some(&self.window),
            gtk::FileChooserAction::Open,
            &[(&gettext("Cancel"), gtk::ResponseType::Cancel), (&gettext("Open"), gtk::ResponseType::Ok)],
        );
        dialog.set_current_folder(glib::home_dir());
        Self::add_filters(&dialog);

        let response = dialog.run();
        let file = dialog.filename();
        unsafe { dialog.destroy() };

        if response == gtk::ResponseType::Ok {
            if let Some(file) = file {
                self.load_notes_from_path(&file, window);
            }
        }
    }

    pub fn restore_backup(&self, window: &gtk::Window) {
        const CLEAR: gtk::ResponseType = gtk::ResponseType::Other(20);

        let dialog = gtk::Dialog::new();
        dialog.set_title(&gettext("Restore Backup"));
        dialog.set_transient_for(Some(window));
        dialog.add_button(&gettext("Cancel"), gtk::ResponseType::Cancel);
        dialog.add_button(&gettext("Clear"), CLEAR);
        let restore_button = dialog.add_button(&gettext("Restore"), gtk::ResponseType::Ok);
        dialog.set_default_response(gtk::ResponseType::Ok);

        let backup_list = gtk::ListBox::new();
        backup_list.set_selection_mode(gtk::SelectionMode::Browse);

        let scrolled = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scrolled.set_shadow_type(gtk::ShadowType::In);
        scrolled.set_min_content_height(200);
        scrolled.add(&backup_list);
        dialog.content_area().pack_start(&scrolled, true, true, 0);

        let backups = list_backups(&config_dir());
        if backups.is_empty() {
            restore_button.set_sensitive(false);
        }

        for name in &backups {
            let stamp: i64 = name[7..name.len() - 5].parse().unwrap_or(0);
            let label_text = glib::DateTime::from_unix_local(stamp)
                .and_then(|d| d.format("%c"))
                .map(|s| s.to_string())
                .unwrap_or_else(|_| name.clone());
            let label = gtk::Label::new(Some(&label_text));
            label.set_margin_top(5);
            label.set_margin_bottom(5);
            label.set_margin_start(5);
            label.set_margin_end(5);
            backup_list.add(&label);
        }

        scrolled.show_all();

        let response = dialog.run();
        let selected = backup_list.selected_row().map(|r| r.index());
        unsafe { dialog.destroy() };

        if response == gtk::ResponseType::Ok {
            if let Some(name) = selected.and_then(|i| backups.get(i as usize)) {
                self.load_notes_from_path(&config_dir().join(name), window);
            }
        } else if response == CLEAR {
            self.delete_all_backups();
        }
    }

    fn load_notes_from_path(&self, path: &Path, window: &gtk::Window) {
        let result = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|d| parse_notes(&d).map_err(|e| e.to_string()));
        match result {
            Ok(lists) => {
                *self.notes_lists.borrow_mut() = lists;
                self.save_note_list();
                self.lists_changed.emit(&());
            }
            Err(_) => show_error(&gettext("Unable to restore: invalid or corrupted backup file"), Some(window)),
        }
    }

    pub fn flush(&self) {
        if let Some(id) = self.save_timer_id.borrow_mut().take() {
            id.remove();
        }
        self.save_note_list();
    }

    pub fn new_group(&self, group_name: &str) -> bool {
        if self.has_group(group_name)
            && !confirm(
                &gettext("Overwrite Existing Group"),
                &gettext_fmt("There is already a group named '%s'. This action will overwrite it. Continue anyway?", group_name),
                Some(&self.window),
                None,
            )
        {
            return false;
        }

        self.notes_lists.borrow_mut().insert(group_name.to_string(), Vec::new());

        self.save_note_list();
        self.lists_changed.emit(&());

        true
    }

    pub fn remove_group(&self, group_name: &str) {
        if !confirm(
            &gettext("Remove Group"),
            &gettext_fmt("Are you sure you want to remove the group %s?", group_name),
            Some(&self.window),
            None,
        ) {
            return;
        }

        if self.notes_lists.borrow_mut().shift_remove(group_name).is_none() {
            eprintln!("sticky: invalid group name {group_name}");
            return;
        }

        self.save_note_list();
        self.lists_changed.emit(&());
    }

    pub fn change_group_name(&self, old_group: &str, new_group: &str) {
        {
            let mut lists = self.notes_lists.borrow_mut();
            let Some(notes) = lists.shift_remove(old_group) else { return };
            lists.insert(new_group.to_string(), notes);
        }

        self.save_note_list();
        self.group_name_changed.emit(&(old_group.to_string(), new_group.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_names() {
        assert!(is_backup_file_name("backup-1700000000.json"));
        assert!(is_backup_file_name("BACKUP-17000000001.JSON"));
        assert!(!is_backup_file_name("backup-170000000.json"));
        assert!(!is_backup_file_name("backup-17000000x0.json"));
        assert!(!is_backup_file_name("notes.json"));
    }
}
