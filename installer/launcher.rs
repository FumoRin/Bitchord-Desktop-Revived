// The pre-reset updater runs the release asset directly from its profile. Move
// this loader out of that tree before starting Inno, and wait for Inno so the
// old updater cannot relaunch BitChord before the reset finishes.
#![windows_subsystem = "windows"]

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::windows::{ffi::{OsStrExt, OsStringExt}, process::CommandExt};
use std::path::{Path, PathBuf};
use std::process::Command;

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetKnownFolderPath(id: *const u32, flags: u32, token: usize, path: *mut *mut u16) -> i32;
}
#[link(name = "ole32")]
unsafe extern "system" {
    fn CoTaskMemFree(ptr: *mut u16);
}
#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(window: usize, text: *const u16, caption: *const u16, flags: u32) -> i32;
}

fn roaming_app_data() -> io::Result<PathBuf> {
    // FOLDERID_RoamingAppData; use the same known folder as the profile reset.
    let id = [0x3eb685db, 0x4cf665f9, 0xefe33aa0, 0x3d9f7265];
    let mut path = std::ptr::null_mut();
    // The API allocates a null-terminated string that the caller must free.
    unsafe {
        let status = SHGetKnownFolderPath(id.as_ptr(), 0, 0, &mut path);
        let result = if status >= 0 {
            let len = (0..).find(|&i| *path.add(i) == 0).unwrap();
            Ok(PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(path, len))))
        } else {
            Err(io::Error::other(format!("could not locate Roaming AppData (0x{status:08x})")))
        };
        CoTaskMemFree(path);
        result
    }
}

fn inside(path: &Path, profile: &Path) -> bool {
    path.ancestors().any(|dir| dir.to_string_lossy().eq_ignore_ascii_case(&profile.to_string_lossy()))
}

fn launch(args: &[OsString]) -> io::Result<i32> {
    let roaming = roaming_app_data()?.canonicalize()?;
    let profile = match roaming.join("bitchord").canonicalize() {
        Ok(path) => path,
        Err(e) if e.kind() == io::ErrorKind::NotFound => roaming.join("bitchord"),
        Err(e) => return Err(e),
    };
    let source = std::env::current_exe()?.canonicalize()?;
    let must_move = inside(&source, &profile);
    let outside = profile.parent().ok_or_else(|| io::Error::other("profile has no parent"))?;
    // Renaming a running exe requires the same volume. Use a sibling of the
    // physical profile for legacy installs, including redirected AppData.
    let temp = std::env::temp_dir().canonicalize().ok().filter(|dir| !inside(dir, &profile))
        .unwrap_or_else(|| outside.to_owned());
    let stage_parent = if must_move { outside } else { &temp };
    let stage = (0..128)
        .map(|n| stage_parent.join(format!("bitchord-setup-{}-{n}", std::process::id())))
        .find_map(|dir| match fs::create_dir(&dir) {
            Ok(()) => Some(Ok(dir)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
            Err(e) => Some(Err(e)),
        })
        .ok_or_else(|| io::Error::other("could not allocate a setup directory"))??;

    let setup = stage.join("setup.exe");
    let moved = stage.join("download.exe");
    let mut relocated = false;
    let result = (|| {
        fs::OpenOptions::new().create_new(true).write(true).open(&setup)?
            .write_all(include_bytes!(env!("BITCHORD_INNO_SETUP")))?;
        if inside(&std::env::current_dir()?.canonicalize()?, &profile) {
            std::env::set_current_dir(&temp)?;
        }
        if must_move {
            fs::rename(&source, &moved)?;
            relocated = true;
        }
        // Inno also extracts its running process into TEMP before installing.
        Command::new(&setup).args(args).env("TEMP", &temp).env("TMP", &temp)
            .status().map(|status| status.code().unwrap_or(1))
    })();
    let _ = fs::remove_file(&setup);
    if relocated && !matches!(result, Ok(0)) && fs::rename(&moved, &source).is_ok() {
        relocated = false;
    }
    if relocated {
        // Windows can rename a mapped executable, but deletion needs this
        // process to exit. Only clean up the two paths this invocation created.
        // cmd's file commands need an ordinary drive or UNC path.
        let wide: Vec<_> = stage.as_os_str().encode_wide().collect();
        let prefix: Vec<_> = r"\\?\".encode_utf16().collect();
        let unc: Vec<_> = r"UNC\".encode_utf16().collect();
        let path = wide.strip_prefix(&prefix[..]).unwrap_or(&wide);
        let path = if let Some(rest) = path.strip_prefix(&unc[..]) {
            OsString::from_wide(&[&[92, 92], rest].concat())
        } else {
            OsString::from_wide(path)
        };
        let _ = Command::new("cmd.exe").args(["/D", "/V:OFF", "/C"])
            .raw_arg(r#"ping -n 3 127.0.0.1 >NUL & del /q "%BITCHORD_SETUP_STAGE%\download.exe" & rd "%BITCHORD_SETUP_STAGE%""#)
            .env("BITCHORD_SETUP_STAGE", path).creation_flags(0x0800_0000).spawn();
    } else {
        let _ = fs::remove_dir(&stage);
    }
    result
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match launch(&args) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            if !args.iter().any(|arg| arg.to_string_lossy().eq_ignore_ascii_case("/verysilent")) {
                let text: Vec<_> = format!("Could not start BitChord Setup: {e}\0").encode_utf16().collect();
                let title: Vec<_> = "BitChord Setup\0".encode_utf16().collect();
                // Both strings are null-terminated and live until the dialog returns.
                unsafe { MessageBoxW(0, text.as_ptr(), title.as_ptr(), 0x10); }
            }
            std::process::exit(1);
        }
    }
}
