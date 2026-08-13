//! XDG autostart manager for SWAI.
//!
//! Writes and removes `~/.config/autostart/swai.desktop` so the desktop
//! session launcher can start SWAI on login.

use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
const DESKTOP_FILENAME: &str = "swai.desktop";

fn autostart_dir() -> PathBuf {
    let home = env::var("HOME").expect("HOME environment variable not set");
    PathBuf::from(home).join(".config").join("autostart")
}

/// Write the XDG autostart `.desktop` file so SWAI launches on login.
pub fn enable_autostart() -> io::Result<()> {
    let dir = autostart_dir();
    fs::create_dir_all(&dir)?;

    let home = env::var("HOME").unwrap_or_default();
    let exec_path = PathBuf::from(&home).join(".local").join("bin").join("swai");
    let exec_cmd = if exec_path.exists() {
        exec_path.to_string_lossy().to_string()
    } else {
        "swai".to_string()
    };

    let desktop_content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=SWAI\n\
         Comment=SWAI Local AI Gateway\n\
         Exec={}\n\
         Terminal=false\n\
         Categories=Utility;Development;\n\
         X-GNOME-Autostart-enabled=true\n",
        exec_cmd
    );

    let path = dir.join(DESKTOP_FILENAME);
    fs::write(path, desktop_content)?;
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
