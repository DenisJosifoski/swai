//! XDG autostart manager for SWAI.
//!
//! Writes and removes `~/.config/autostart/swai.desktop` so the desktop
//! session launcher can start SWAI on login.

use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

const DESKTOP_FILENAME: &str = "swai.desktop";
const DESKTOP_ENTRY: &str = "\
[Desktop Entry]
Type=Application
Name=SWAI
Comment=SWAI Local AI Gateway
Exec=swai
NoDisplay=true
X-GNOME-Autostart-enabled=true
";

fn autostart_dir() -> PathBuf {
    let home = env::var("HOME").expect("HOME environment variable not set");
    PathBuf::from(home).join(".config").join("autostart")
}

/// Write the XDG autostart `.desktop` file so SWAI launches on login.
///
/// Creates `~/.config/autostart/swai.desktop` if it does not already exist,
/// overwriting any stale copy. Returns an error if the directory cannot be
/// created or the file cannot be written.
pub fn enable_autostart() -> io::Result<()> {
    let dir = autostart_dir();
    fs::create_dir_all(&dir)?;

    let path = dir.join(DESKTOP_FILENAME);
    fs::write(path, DESKTOP_ENTRY)?;
    Ok(())
}

/// Remove the XDG autostart `.desktop` file so SWAI no longer launches on login.
///
/// Silently ignores the case where the file does not already exist.
pub fn disable_autostart() -> io::Result<()> {
    let path = autostart_dir().join(DESKTOP_FILENAME);
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn with_home<F: FnOnce()>(f: F) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let old = env::var("HOME").ok();
        env::set_var("HOME", &home);
        f();
        if let Some(old) = old {
            env::set_var("HOME", old);
        } else {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_enable_creates_desktop_file() {
        with_home(|| {
            enable_autostart().unwrap();
            let path = autostart_dir().join(DESKTOP_FILENAME);
            assert!(path.exists());
            let content = fs::read_to_string(&path).unwrap();
            assert!(content.contains("Type=Application"));
            assert!(content.contains("Exec=swai"));
            assert!(content.contains("Name=SWAI"));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_disable_removes_desktop_file() {
        with_home(|| {
            enable_autostart().unwrap();
            disable_autostart().unwrap();
            let path = autostart_dir().join(DESKTOP_FILENAME);
            assert!(!path.exists());
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_disable_without_file_is_ok() {
        with_home(|| {
            // No file present — should not error.
            disable_autostart().unwrap();
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_enable_overwrites_existing_file() {
        with_home(|| {
            enable_autostart().unwrap();
            // Write something different to the same path.
            fs::write(autostart_dir().join(DESKTOP_FILENAME), "stale\n").unwrap();
            enable_autostart().unwrap();
            let content = fs::read_to_string(autostart_dir().join(DESKTOP_FILENAME)).unwrap();
            assert!(content.contains("Exec=swai"));
        });
    }
}
