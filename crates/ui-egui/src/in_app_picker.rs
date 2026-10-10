//! File pickers drawn by PdfCraft itself, for iPadOS.
//!
//! iPadOS has no `rfd` backend, and presenting UIKit's document picker needs Objective-C classes
//! (`unsafe`, forbidden here). So on iPad every picker the UI asks for ([`crate::pickers`]) is a
//! modal listing the app's Documents folder, which the Files app shows as On My iPad ▸ PdfCraft:
//! files put there in Files (or shared to PdfCraft, see `apps/pdfcraft-ios`) can be opened,
//! inserted and combined, and saves land there.
//!
//! The picker can't leave the Documents folder: [`InAppPick::enter`] refuses anything outside it,
//! and names typed into Save are single path components ([`clean_file_name`]).
//!
//! [`AsyncFileDialog`] has the subset of `rfd::AsyncFileDialog`'s builder the UI uses, so call
//! sites are the same on every platform ([`crate::dialog`]).

// The picker only runs on iPadOS; the desktop builds compile it for its tests.
#![cfg_attr(not(target_os = "ios"), allow(dead_code))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use egui::{Align, Layout, RichText};

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, widgets};

/// Entries listed in one folder at most; a folder with more shows the first ones (sorted) and
/// says so.
const MAX_ENTRIES: usize = 2000;

/// The longest file name accepted, in bytes (APFS allows 255 UTF-8 bytes).
const MAX_NAME_BYTES: usize = 255;

/// What the picker was asked for: a description built like `rfd::AsyncFileDialog`.
#[derive(Clone, Debug, Default)]
pub struct AsyncFileDialog {
    title: Option<String>,
    filters: Vec<(String, Vec<String>)>,
    file_name: Option<String>,
}

impl AsyncFileDialog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Only list files with one of these extensions (several filters: any of them).
    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.filters.push((name.into(), extensions.iter().map(|e| e.to_string().to_ascii_lowercase()).collect()));
        self
    }

    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// The name Save suggests.
    pub fn set_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }

    /// Whether `path` passes the filters (no filters: every file).
    fn accepts(&self, path: &Path) -> bool {
        if self.filters.is_empty() {
            return true;
        }
        let Some(ext) = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()) else { return false };
        self.filters.iter().any(|(_, exts)| exts.iter().any(|e| e == "*" || *e == ext))
    }

    /// The extension Save adds to a name typed without one of the filters' extensions.
    fn default_extension(&self) -> Option<&str> {
        self.filters.first().and_then(|(_, exts)| exts.iter().find(|e| *e != "*")).map(String::as_str)
    }
}

/// Which kind of picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Files,
    Folder,
    Save,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    name: String,
    path: PathBuf,
    folder: bool,
}

/// A picker on screen.
pub struct InAppPick {
    pub(crate) kind: Kind,
    dialog: AsyncFileDialog,
    root: PathBuf,
    dir: PathBuf,
    entries: Vec<Entry>,
    truncated: bool,
    error: Option<String>,
    chosen: BTreeSet<PathBuf>,
    /// The name being typed (Save).
    name: String,
    /// Save asked to replace this existing file; the next Save confirms.
    replace: Option<PathBuf>,
}

/// How a frame of the picker ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Still choosing.
    Open,
    /// Done: the chosen paths (none when cancelled).
    Done(Vec<PathBuf>),
}

impl InAppPick {
    pub fn new(kind: Kind, dialog: AsyncFileDialog, root: PathBuf) -> Self {
        let name = dialog.file_name.as_deref().and_then(clean_file_name).unwrap_or_default();
        let mut pick = Self {
            kind,
            dialog,
            dir: root.clone(),
            root,
            entries: Vec::new(),
            truncated: false,
            error: None,
            chosen: BTreeSet::new(),
            name,
            replace: None,
        };
        pick.refresh();
        pick
    }

    /// List the current folder again: folders first, then the files the filters accept, each
    /// sorted by name without regard to case. Hidden files and the system's `Inbox` are left out.
    pub fn refresh(&mut self) {
        self.entries.clear();
        self.truncated = false;
        self.error = None;
        let read = match std::fs::read_dir(&self.dir) {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        for entry in read.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || (self.dir == self.root && name == "Inbox") {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            let folder = kind.is_dir();
            let listed = if folder { true } else { kind.is_file() && self.kind != Kind::Folder && self.dialog.accepts(&path) };
            if !listed {
                continue;
            }
            if self.entries.len() >= MAX_ENTRIES {
                self.truncated = true;
                break;
            }
            self.entries.push(Entry { name, path, folder });
        }
        self.entries.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    }

    /// Go into `dir`; refused (false) when it isn't inside the Documents folder.
    pub fn enter(&mut self, dir: &Path) -> bool {
        if !inside(&self.root, dir) {
            return false;
        }
        self.dir = dir.to_path_buf();
        self.chosen.clear();
        self.replace = None;
        self.refresh();
        true
    }

    /// The path Save writes, from the typed name; `None` while the name isn't usable.
    pub fn save_path(&self) -> Option<PathBuf> {
        let mut name = clean_file_name(&self.name)?;
        let typed_ext = Path::new(&name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        let known = typed_ext.as_deref().is_some_and(|e| self.dialog.filters.iter().any(|(_, exts)| exts.iter().any(|x| x == e)));
        if !known && let Some(ext) = self.dialog.default_extension() {
            name = format!("{name}.{ext}");
            if name.len() > MAX_NAME_BYTES {
                return None;
            }
        }
        Some(self.dir.join(name))
    }

    /// Save pressed: the path to write, or `None` after asking to replace an existing file
    /// (pressing Save again with the same name confirms).
    pub fn confirm_save(&mut self) -> Option<PathBuf> {
        let path = self.save_path()?;
        if path.exists() && self.replace.as_ref() != Some(&path) {
            self.replace = Some(path);
            return None;
        }
        Some(path)
    }

    /// The breadcrumb from Documents down to the current folder: (label, folder) pairs.
    fn crumbs(&self) -> Vec<(String, PathBuf)> {
        let mut out = vec![(tl!("Documents").to_string(), self.root.clone())];
        if let Ok(rest) = self.dir.strip_prefix(&self.root) {
            let mut at = self.root.clone();
            for part in rest.components() {
                at.push(part);
                out.push((part.as_os_str().to_string_lossy().into_owned(), at.clone()));
            }
        }
        out
    }

    fn title(&self) -> String {
        if let Some(t) = &self.dialog.title {
            return t.clone();
        }
        match self.kind {
            Kind::File | Kind::Files => tl!("Open…"),
            Kind::Folder => tl!("Choose this folder"),
            Kind::Save => tl!("Save as…"),
        }
        .to_string()
    }

    /// Draw the picker for one frame.
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let t = Tokens::get(ctx);
        let mut outcome = Outcome::Open;
        let row_h = crate::touch::row_height(ctx);
        let screen = ctx.content_rect();
        let modal = egui::Modal::new(egui::Id::new("in_app_picker")).show(ctx, |ui| {
            ui.set_width((screen.width() - 48.0).clamp(280.0, 620.0));
            ui.horizontal(|ui| {
                ui.add(crate::icons::image("folder-open", 22.0, t.accent));
                ui.label(RichText::new(self.title()).font(theme::semibold(18.0)));
            });
            ui.add_space(6.0);
            // Breadcrumb: tap a folder to go back up to it.
            let mut go: Option<PathBuf> = None;
            ui.horizontal_wrapped(|ui| {
                let crumbs = self.crumbs();
                let last = crumbs.len().saturating_sub(1);
                for (i, (label, dir)) in crumbs.into_iter().enumerate() {
                    if i > 0 {
                        ui.add(crate::icons::image("chevron-right", 14.0, t.text_faint));
                    }
                    if i == last {
                        ui.label(RichText::new(label).strong());
                    } else if ui.link(label).clicked() {
                        go = Some(dir);
                    }
                }
            });
            ui.separator();
            let list_h = (screen.height() * 0.55).clamp(160.0, 560.0);
            egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
                if let Some(e) = &self.error {
                    let name = self.dir.file_name().map_or_else(|| tl!("Documents").to_string(), |n| n.to_string_lossy().into_owned());
                    let text = crate::i18n::fmt(tl!("Couldn't read {name}: {e}"), &[("name", &name), ("e", e)]);
                    ui.label(RichText::new(text).color(ui.visuals().error_fg_color));
                } else if self.entries.is_empty() {
                    ui.add_space(12.0);
                    ui.label(RichText::new(tl!("Nothing here yet. Put files in Files › On My iPad › PdfCraft.")).color(t.text_muted));
                }
                let mut picked: Option<PathBuf> = None;
                for entry in &self.entries {
                    let selected = self.chosen.contains(&entry.path);
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), egui::Sense::click());
                    if selected || resp.hovered() {
                        ui.painter().rect_filled(rect, 6.0, if selected { t.accent_soft } else { t.hover });
                    }
                    let icon = if entry.folder { "folder" } else { "file-text" };
                    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), egui::vec2(20.0, 20.0));
                    crate::icons::paint(ui, icon_rect, icon, 20.0, if entry.folder { t.accent } else { t.text_muted });
                    widgets::row_text(
                        ui,
                        egui::pos2(rect.left() + 40.0, rect.center().y),
                        &entry.name,
                        theme::regular(15.0),
                        t.text,
                        rect.width() - 48.0,
                    );
                    if resp.clicked() {
                        picked = Some(entry.path.clone());
                    }
                }
                if self.truncated {
                    // More than MAX_ENTRIES: the rest aren't listed.
                    ui.label(RichText::new("…").color(t.text_muted));
                }
                if let Some(path) = picked {
                    let folder = self.entries.iter().any(|e| e.path == path && e.folder);
                    if folder {
                        go = Some(path);
                    } else {
                        match self.kind {
                            Kind::File => outcome = Outcome::Done(vec![path]),
                            Kind::Files => {
                                if !self.chosen.remove(&path) {
                                    self.chosen.insert(path);
                                }
                            }
                            Kind::Save => {
                                self.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                self.replace = None;
                            }
                            Kind::Folder => {}
                        }
                    }
                }
            });
            if let Some(dir) = go {
                self.enter(&dir);
            }
            ui.separator();
            if self.kind == Kind::Save {
                ui.horizontal(|ui| {
                    let l = ui.label(tl!("Name:"));
                    let edit = ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(8, 6)));
                    if edit.changed() {
                        self.replace = None;
                    }
                    edit.labelled_by(l.id);
                });
                if let Some(path) = &self.replace {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(
                        RichText::new(crate::i18n::fmt(tl!("“{name}” already exists. Save again to replace it."), &[("name", &name)]))
                            .color(ui.visuals().error_fg_color),
                    );
                }
            }
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match self.kind {
                    Kind::Save => {
                        let ok = self.save_path().is_some();
                        if ui.add_enabled_ui(ok, |ui| widgets::pill_button(ui, tl!("Save"), true)).inner.clicked()
                            && let Some(path) = self.confirm_save()
                        {
                            outcome = Outcome::Done(vec![path]);
                        }
                    }
                    Kind::Files => {
                        let n = self.chosen.len();
                        let label = format!("{} ({n})", tl!("Add"));
                        if ui.add_enabled_ui(n > 0, |ui| widgets::pill_button(ui, &label, true)).inner.clicked() {
                            outcome = Outcome::Done(self.chosen.iter().cloned().collect());
                        }
                    }
                    Kind::Folder => {
                        if widgets::pill_button(ui, tl!("Choose this folder"), true).clicked() {
                            outcome = Outcome::Done(vec![self.dir.clone()]);
                        }
                    }
                    Kind::File => {}
                }
                if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                    outcome = Outcome::Done(Vec::new());
                }
            });
        });
        if outcome == Outcome::Open && modal.should_close() {
            outcome = Outcome::Done(Vec::new());
        }
        outcome
    }
}

/// Whether `path` is `root` or inside it, judged on the path's components (no `..`), so a
/// crafted path can't climb out.
fn inside(root: &Path, path: &Path) -> bool {
    use std::path::Component;
    path.strip_prefix(root).is_ok_and(|rest| rest.components().all(|c| matches!(c, Component::Normal(_))))
}

/// `name` as one file name: trimmed, no separators or NUL, not `.`/`..`, at most
/// [`MAX_NAME_BYTES`]. `None` when nothing usable is left.
pub fn clean_file_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.len() > MAX_NAME_BYTES || name.chars().any(|c| c == '/' || c == '\\' || c == '\0') {
        return None;
    }
    Some(name.to_string())
}

impl PdfCraftApp {
    /// Draw the in-app picker, if one is showing, and hand its choice to the frame that uses
    /// picks ([`PdfCraftApp::process_picked`]).
    pub(crate) fn show_in_app_picker(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.in_app_pick.as_mut() else { return };
        if let Outcome::Done(paths) = pending.pick.show(ctx)
            && let Some(done) = self.in_app_pick.take()
        {
            self.pickers.deliver(done.target, paths, done.then);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdfcraft-picker-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Projects/2026")).unwrap();
        std::fs::create_dir_all(dir.join("Inbox")).unwrap();
        for f in ["b.pdf", "A.PDF", "notes.txt", ".hidden.pdf", "Projects/plan.pdf"] {
            std::fs::write(dir.join(f), b"%PDF-1.7").unwrap();
        }
        dir
    }

    fn names(p: &InAppPick) -> Vec<&str> {
        p.entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn lists_folders_then_matching_files_and_hides_the_rest() {
        let root = scratch("list");
        let pick = InAppPick::new(Kind::File, AsyncFileDialog::new().add_filter("PDF", &["pdf"]), root.clone());
        assert_eq!(names(&pick), vec!["Projects", "A.PDF", "b.pdf"]);
        let folders = InAppPick::new(Kind::Folder, AsyncFileDialog::new(), root.clone());
        assert_eq!(names(&folders), vec!["Projects"]);
        let any = InAppPick::new(Kind::File, AsyncFileDialog::new(), root.clone());
        assert_eq!(names(&any), vec!["Projects", "A.PDF", "b.pdf", "notes.txt"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn never_leaves_the_documents_folder() {
        let root = scratch("escape");
        let mut pick = InAppPick::new(Kind::File, AsyncFileDialog::new(), root.clone());
        assert!(pick.enter(&root.join("Projects")));
        assert_eq!(names(&pick), vec!["2026", "plan.pdf"]);
        assert_eq!(pick.crumbs().len(), 2);
        assert!(!pick.enter(&root.join("Projects/../..")));
        assert!(!pick.enter(Path::new("/")));
        assert!(!pick.enter(root.parent().unwrap()));
        assert_eq!(pick.dir, root.join("Projects"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn save_adds_the_extension_and_asks_before_replacing() {
        let root = scratch("save");
        let dialog = AsyncFileDialog::new().add_filter("PDF", &["pdf"]).set_file_name("Report");
        let mut pick = InAppPick::new(Kind::Save, dialog, root.clone());
        assert_eq!(pick.save_path(), Some(root.join("Report.pdf")));
        pick.name = "b".into();
        assert_eq!(pick.confirm_save(), None, "b.pdf exists: ask first");
        assert!(pick.replace.is_some());
        assert_eq!(pick.confirm_save(), Some(root.join("b.pdf")), "the second Save replaces");
        for bad in ["", "  ", "..", "a/b", "../x", "x\0y"] {
            pick.name = bad.into();
            assert_eq!(pick.save_path(), None, "{bad:?}");
        }
        pick.name = "é".repeat(200);
        assert_eq!(pick.save_path(), None, "too long for the file system");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_folder_is_reported_not_a_crash() {
        let pick = InAppPick::new(Kind::File, AsyncFileDialog::new(), std::env::temp_dir().join("pdfcraft-picker-missing"));
        assert!(pick.error.is_some());
        assert!(pick.entries.is_empty());
    }
}
