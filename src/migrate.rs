//! The one-time move to the app's own data folder and credential service.
//!
//! Until 2.0.3 the fork kept ytm-rs's identity on disk: its data folder (settings,
//! queue, downloads, sign-in fallbacks) and the credential-store service its sign-ins
//! lived under were both named ytm-rs, so the two apps overwrote each other's
//! settings and fought over one login. Since 2.0.3 the app has its own. This module
//! moves an existing install over, once, on the first launch of the new version:
//! everything is *copied*, never moved or deleted — a parallel ytm-rs install keeps
//! its folder and its sign-in untouched.

use std::path::{Path, PathBuf};

/// The project name every version before 2.0.3 kept its data under, inherited from
/// ytm-rs.
const OLD_PROJECT: &str = "ytm-rs";

/// The data folder those versions used, if there is one.
fn old_data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", OLD_PROJECT).map(|d| d.data_dir().to_path_buf()).filter(|d| d.is_dir())
}

/// Move an existing install onto `dir` (the default data folder) once: the old
/// folder's contents are copied in, and the sign-ins kept under the old credential
/// service are imported. A dev profile (`YTM_DATA_DIR`) or a portable copy keeps
/// nothing of the sort and is left alone.
pub fn run(dir: &Path) {
    if std::env::var_os("YTM_DATA_DIR").is_some() || crate::update::portable_dir().is_some() {
        return;
    }
    // The marker makes this once per install: without it, a sign-out here would be
    // undone by the next launch finding the old service's entry again.
    let marker = dir.join(".migrated-from-ytm-rs");
    if marker.exists() {
        return;
    }
    if let Some(old) = old_data_dir() {
        let moved = copy_dir(&old, dir);
        if moved > 0 {
            log::info!("moved {moved} files over from {}", old.display());
        }
    }
    crate::vault::import_legacy();
    if let Err(e) = std::fs::write(&marker, b"") {
        log::warn!("couldn't write {}: {e}", marker.display());
    }
}

/// Copy `from` into `to`, keeping what's already there, and say how many files went
/// over. Staged installers (`updates`) and old logs don't carry: the first is
/// version-specific and re-downloads, the second belongs to the run that wrote it.
fn copy_dir(from: &Path, to: &Path) -> usize {
    let mut n = 0;
    let Ok(entries) = std::fs::read_dir(from) else { return 0 };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == "updates" || name.to_string_lossy().ends_with(".log") {
            continue;
        }
        let (source, dest) = (entry.path(), to.join(&name));
        if source.is_dir() {
            let _ = std::fs::create_dir_all(&dest);
            n += copy_dir(&source, &dest);
        } else if !dest.exists() {
            // fs::copy carries the permissions too: a sign-in fallback file is
            // owner-only and stays that way.
            match std::fs::copy(&source, &dest) {
                Ok(_) => n += 1,
                Err(e) => log::warn!("couldn't copy {} over: {e}", source.display()),
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The move-over keeps what's already there, skips the staged installers and the
    /// logs, and recurses.
    #[test]
    fn copies_files_but_not_staged_installers_or_logs() {
        let root = std::env::temp_dir().join(format!("bitchord-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (old, new) = (root.join("old"), root.join("new"));
        std::fs::create_dir_all(old.join("downloads")).unwrap();
        std::fs::create_dir_all(old.join("updates")).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("app.ron"), "settings").unwrap();
        std::fs::write(old.join("google session.secret"), b"cookie").unwrap();
        std::fs::write(old.join("bitchord.log"), "last run").unwrap();
        std::fs::write(old.join("downloads/a.song"), b"audio").unwrap();
        std::fs::write(old.join("updates/bitchord-setup-2.0.2.exe"), b"installer").unwrap();
        // A file the new folder already has is not overwritten.
        std::fs::write(new.join("app.ron"), "newer settings").unwrap();

        // Three files to move, one of which the new folder already has: two go over.
        assert_eq!(copy_dir(&old, &new), 2);
        assert_eq!(std::fs::read_to_string(new.join("app.ron")).unwrap(), "newer settings");
        assert_eq!(std::fs::read(new.join("google session.secret")).unwrap(), b"cookie");
        assert_eq!(std::fs::read(new.join("downloads/a.song")).unwrap(), b"audio");
        assert!(!new.join("bitchord.log").exists());
        assert!(!new.join("updates").exists());

        // A second pass copies nothing: everything there is already there.
        assert_eq!(copy_dir(&old, &new), 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
