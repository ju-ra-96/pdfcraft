//! The app's Documents folder, where PdfCraft for iPad keeps the files it opens and saves.
//!
//! iPadOS gives every app a sandbox. `UIFileSharingEnabled` in `Info.plist` shows its Documents
//! folder in the Files app (On My iPad ▸ PdfCraft), so files dragged there in Files or the Finder
//! are what the in-app browser lists. A PDF shared from another app ("Open in PdfCraft", AirDrop)
//! is copied by the system into `Documents/Inbox`; [`Inbox::take`] moves each one up into
//! Documents, where it can be saved in place, and hands it to the UI to open.
//!
//! Plain file-system calls only: no UIKit (which would need `unsafe`, forbidden here).

// Only iPadOS starts the app; the desktop build compiles this module for its tests.
#![cfg_attr(not(target_os = "ios"), allow(dead_code))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often the Inbox is looked at. Cheap (one `read_dir` of a folder that is nearly always
/// empty), but it runs on the UI thread, so not every frame.
pub const POLL_EVERY: Duration = Duration::from_millis(750);

/// At most this many shared files are opened at once; the rest wait for the next poll, so a
/// share of thousands of files can't open thousands of tabs in one frame.
const MAX_PER_POLL: usize = 16;

/// The sandbox folders, from `HOME` (the app container on iPadOS).
#[derive(Clone, Debug)]
pub struct Folders {
    pub documents: PathBuf,
    pub settings: PathBuf,
}

impl Folders {
    pub fn from_home(home: &Path) -> Self {
        Self { documents: home.join("Documents"), settings: home.join("Library").join("Application Support").join("PdfCraft") }
    }

    pub fn current() -> Option<Self> {
        let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from)?;
        Some(Self::from_home(&home))
    }

    /// Create the folders (best effort: a failure is logged, and the app carries on without
    /// persistence rather than not starting).
    pub fn create(&self) {
        for dir in [&self.documents, &self.settings] {
            if let Err(e) = std::fs::create_dir_all(dir) {
                log::warn!("could not create {}: {e}", dir.display());
            }
        }
    }
}

/// Watches `Documents/Inbox` for files shared from other apps.
pub struct Inbox {
    documents: PathBuf,
    last_poll: Option<Instant>,
    /// Files that couldn't be moved out of the Inbox, so they aren't reported every poll.
    stuck: BTreeSet<PathBuf>,
}

impl Inbox {
    pub fn new(documents: PathBuf) -> Self {
        Self { documents, last_poll: None, stuck: BTreeSet::new() }
    }

    /// The PDFs that arrived since the last call, already moved into Documents, or nothing if
    /// the last poll was less than [`POLL_EVERY`] ago.
    pub fn poll(&mut self, now: Instant) -> Vec<PathBuf> {
        if self.last_poll.is_some_and(|t| now.saturating_duration_since(t) < POLL_EVERY) {
            return Vec::new();
        }
        self.last_poll = Some(now);
        self.take()
    }

    /// Move every PDF out of `Documents/Inbox` into Documents (renamed `name 2.pdf`, `name 3.pdf`,
    /// … if the name is taken) and return the new paths. Anything that isn't a regular `.pdf`
    /// file is left where it is.
    pub fn take(&mut self) -> Vec<PathBuf> {
        let inbox = self.documents.join("Inbox");
        let Ok(entries) = std::fs::read_dir(&inbox) else { return Vec::new() };
        let mut found: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.path())
            .filter(|p| is_pdf(p) && !self.stuck.contains(p))
            .collect();
        found.sort();
        let mut moved = Vec::new();
        for from in found.into_iter().take(MAX_PER_POLL) {
            let Some(name) = from.file_name() else { continue };
            let to = free_name(&self.documents, Path::new(name));
            match std::fs::rename(&from, &to) {
                Ok(()) => moved.push(to),
                Err(e) => {
                    log::warn!("could not move shared file {} into Documents: {e}", from.display());
                    self.stuck.insert(from);
                }
            }
        }
        moved
    }
}

pub fn is_pdf(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// `dir/name`, or `dir/stem 2.ext`, `dir/stem 3.ext`, … for the first name nothing has yet.
pub fn free_name(dir: &Path, name: &Path) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let stem = name.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Document".into());
    let ext = name.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    // Bounded: a folder can't hold this many copies, and a bound keeps a broken file system
    // (`exists` always true) from looping forever.
    for n in 2..10_000u32 {
        let candidate = dir.join(format!("{stem} {n}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} {}{ext}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdfcraft-ios-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Inbox")).unwrap();
        dir
    }

    #[test]
    fn shared_pdfs_move_out_of_the_inbox_without_overwriting() {
        let docs = scratch("inbox");
        std::fs::write(docs.join("Report.pdf"), b"mine").unwrap();
        std::fs::write(docs.join("Inbox/Report.pdf"), b"shared").unwrap();
        std::fs::write(docs.join("Inbox/notes.txt"), b"not a pdf").unwrap();
        std::fs::write(docs.join("Inbox/Scan.PDF"), b"upper-case extension").unwrap();

        let mut inbox = Inbox::new(docs.clone());
        let moved = inbox.take();
        assert_eq!(moved, vec![docs.join("Report 2.pdf"), docs.join("Scan.PDF")]);
        assert_eq!(std::fs::read(docs.join("Report.pdf")).unwrap(), b"mine");
        assert_eq!(std::fs::read(docs.join("Report 2.pdf")).unwrap(), b"shared");
        assert!(docs.join("Inbox/notes.txt").exists(), "only PDFs are taken");
        assert!(inbox.take().is_empty(), "each file is reported once");
        let _ = std::fs::remove_dir_all(&docs);
    }

    #[test]
    fn polling_is_throttled() {
        let docs = scratch("poll");
        let mut inbox = Inbox::new(docs.clone());
        let t0 = Instant::now();
        assert!(inbox.poll(t0).is_empty());
        std::fs::write(docs.join("Inbox/a.pdf"), b"%PDF").unwrap();
        assert!(inbox.poll(t0 + Duration::from_millis(10)).is_empty(), "too soon");
        assert_eq!(inbox.poll(t0 + POLL_EVERY).len(), 1);
        let _ = std::fs::remove_dir_all(&docs);
    }

    #[test]
    fn a_missing_inbox_is_not_an_error() {
        let mut inbox = Inbox::new(std::env::temp_dir().join("pdfcraft-ios-no-such-folder"));
        assert!(inbox.take().is_empty());
    }

    #[test]
    fn folders_live_in_the_container() {
        let f = Folders::from_home(Path::new("/var/mobile/Containers/Data/Application/X"));
        assert_eq!(f.documents, Path::new("/var/mobile/Containers/Data/Application/X/Documents"));
        assert!(f.settings.ends_with("Library/Application Support/PdfCraft"));
    }
}
