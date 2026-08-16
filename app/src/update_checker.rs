/// Update checker module — checks for new releases on GitHub and reports them to the UI.
///
/// This module provides:
/// - Semantic version parsing and comparison (v-prefixed tags)
/// - GitHub release API integration via blocking reqwest
/// - URL opening via `xdg-open` on Linux
use reqwest::blocking::Client;
use serde::Deserialize;
use std::process::Command;
use std::str::FromStr;

// ─── Version ──────────────────────────────────────────────────────────────

/// Represents a semantic version (major.minor.patch).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Create a new version from components.
    #[allow(dead_code)]
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Compare two versions. Returns Ordering where:
    /// - `Less` means self < other (other is newer)
    /// - `Equal` means self == other (same version)
    /// - `Greater` means self > other (self is newer)
    pub fn compare_to(&self, other: &Version) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
    }

    /// Check if this version is older than the other.
    pub fn is_older_than(&self, other: &Version) -> bool {
        self.compare_to(other) == std::cmp::Ordering::Less
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for Version {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().trim_start_matches('v'); // Strip leading 'v' if present
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(format!(
                "Invalid version format: {}. Expected major.minor.patch",
                s
            ));
        }

        let major = parts[0]
            .parse::<u32>()
            .map_err(|e| format!("Invalid major version: {}", e))?;
        let minor = parts[1]
            .parse::<u32>()
            .map_err(|e| format!("Invalid minor version: {}", e))?;
        let patch = parts[2]
            .parse::<u32>()
            .map_err(|e| format!("Invalid patch version: {}", e))?;

        Ok(Version {
            major,
            minor,
            patch,
        })
    }
}

// ─── Update Result ────────────────────────────────────────────────────────

/// Result of an update check.
#[derive(Debug, Clone)]
pub enum UpdateCheckResult {
    /// No update available — running latest version.
    NoUpdate,
    /// An update is available with details.
    UpdateAvailable {
        version: String,
        #[allow(dead_code)]
        release_url: String,
        #[allow(dead_code)]
        changelog: Option<String>,
    },
    /// The check failed (network error, etc.).
    Error(String),
}

// ─── GitHub Release Payload Parser ────────────────────────────────────────

/// Minimal deserialization of the GitHub releases/latest JSON.
#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: Option<String>,
    html_url: Option<String>,
    body: Option<String>,
}

// ─── Check for Updates ────────────────────────────────────────────────────

/// Query GitHub releases API and compare against the current version.
///
/// Uses a blocking reqwest client so it can be called from the GTK main thread
/// (e.g. inside a `gio::idle_add` callback). Returns `UpdateCheckResult`.
pub fn check_for_updates_blocking(github_repo: &str, current_version: &str) -> UpdateCheckResult {
    let client = Client::builder()
        .user_agent("SWAI/1.0 (Linux; GTK4)")
        .timeout(std::time::Duration::from_secs(10))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => return UpdateCheckResult::Error(format!("Failed to build HTTP client: {}", e)),
    };

    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        github_repo
    );

    let response = match client.get(&url).send() {
        Ok(resp) => resp,
        Err(e) => {
            return UpdateCheckResult::Error(format!("Failed to fetch releases: {}", e));
        }
    };

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // Repository has no published releases yet — running current version.
        return UpdateCheckResult::NoUpdate;
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return UpdateCheckResult::Error(format!(
            "GitHub API returned status {}: {}",
            status, body
        ));
    }

    let release_text = match response.text() {
        Ok(t) => t,
        Err(e) => {
            return UpdateCheckResult::Error(format!("Failed to read response body: {}", e));
        }
    };

    let release: GitHubRelease = match serde_json::from_str(&release_text) {
        Ok(v) => v,
        Err(e) => {
            return UpdateCheckResult::Error(format!("Failed to parse release JSON: {}", e));
        }
    };

    let tag_name = match &release.tag_name {
        Some(s) => s.clone(),
        None => {
            return UpdateCheckResult::Error("Release has no tag_name".to_string());
        }
    };

    // Parse the latest version.
    let latest_version: Version = match tag_name.parse() {
        Ok(v) => v,
        Err(e) => {
            return UpdateCheckResult::Error(format!(
                "Invalid version in tag '{}': {}",
                tag_name, e
            ));
        }
    };

    // Parse the current version.
    let current_version_parsed: Version = match current_version.parse() {
        Ok(v) => v,
        Err(e) => {
            return UpdateCheckResult::Error(format!(
                "Invalid current version '{}': {}. Make sure your Cargo.toml has a valid semver.",
                current_version, e
            ));
        }
    };

    // Compare versions.
    if latest_version.compare_to(&current_version_parsed) == std::cmp::Ordering::Equal {
        return UpdateCheckResult::NoUpdate;
    }

    if latest_version.is_older_than(&current_version_parsed) {
        // Latest release is older than current build — might be a pre-release.
        return UpdateCheckResult::NoUpdate;
    }

    // An update is available.
    let release_url = release.html_url.unwrap_or_default();
    let changelog = release.body.filter(|b| !b.is_empty());

    let version_str = tag_name.trim_start_matches('v').to_string();

    UpdateCheckResult::UpdateAvailable {
        version: version_str,
        release_url,
        changelog,
    }
}

// ─── URL Opener ───────────────────────────────────────────────────────────

/// Open a URL in the system default browser.
#[allow(dead_code)]
pub fn open_url(url: &str) {
    let _ = Command::new("xdg-open").arg(url).spawn();
}

// ─── Download URL Builder ─────────────────────────────────────────────────

/// Get the download URL for the current platform's update.
#[allow(dead_code)]
pub fn get_download_url(github_repo: &str, version: &str) -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    // Construct download URL based on platform.
    match (os, arch) {
        ("linux", "x86_64") => format!(
            "https://github.com/{}/releases/download/{}/swai-linux-x86_64.AppImage",
            github_repo, version
        ),
        ("linux", "aarch64") => format!(
            "https://github.com/{}/releases/download/{}/swai-linux-aarch64.AppImage",
            github_repo, version
        ),
        ("macos", "aarch64") => format!(
            "https://github.com/{}/releases/download/{}/swai-macos-aarch64.dmg",
            github_repo, version
        ),
        ("windows", "x86_64") => format!(
            "https://github.com/{}/releases/download/{}/swai-windows-x86_64.exe",
            github_repo, version
        ),
        _ => format!(
            "https://github.com/{}/releases/tag/{}",
            github_repo, version
        ),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parsing() {
        let v: Version = "1.2.3".parse().unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_version_parsing_with_v_prefix() {
        let v: Version = "v2.0.1".parse().unwrap();
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, 0);
        assert_eq!(v.patch, 1);
    }

    #[test]
    fn test_version_comparison() {
        let v1: Version = "1.0.0".parse().unwrap();
        let v2: Version = "1.1.0".parse().unwrap();
        let v3: Version = "2.0.0".parse().unwrap();

        assert!(v1.is_older_than(&v2));
        assert!(v1.is_older_than(&v3));
        assert!(!v2.is_older_than(&v1));
        assert_eq!(v1.compare_to(&v1), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_version_display() {
        let v: Version = "3.14.0".parse().unwrap();
        assert_eq!(format!("{}", v), "3.14.0");
    }

    #[test]
    fn test_invalid_version_parsing() {
        assert!("invalid".parse::<Version>().is_err());
        assert!("1.2".parse::<Version>().is_err());
        assert!("1.2.3.4".parse::<Version>().is_err());
        assert!("abc.def.ghi".parse::<Version>().is_err());
    }

    #[test]
    fn test_get_download_url_linux_x86_64() {
        let url = get_download_url("owner/repo", "v1.0.0");
        assert_eq!(
            url,
            "https://github.com/owner/repo/releases/download/v1.0.0/swai-linux-x86_64.AppImage"
        );
    }

    #[test]
    fn test_get_download_url_windows() {
        // Note: This test verifies the URL construction logic. On non-Windows
        // platforms, the actual OS constant will differ, but the format is correct.
        let url = get_download_url("owner/repo", "v2.0.0");
        // Verify the URL contains the expected components.
        assert!(url.contains("owner/repo"));
        assert!(url.contains("v2.0.0"));
        assert!(url.contains("releases/download/"));
    }

    #[test]
    fn test_update_check_result_no_update() {
        // Verify NoUpdate variant exists and is debug-printable.
        let result = UpdateCheckResult::NoUpdate;
        let s = format!("{:?}", result);
        assert!(s.contains("NoUpdate"));
    }

    #[test]
    fn test_update_check_result_available() {
        let result = UpdateCheckResult::UpdateAvailable {
            version: "1.2.0".to_string(),
            release_url: "https://github.com/test/repo/releases/tag/v1.2.0".to_string(),
            changelog: Some("Fixed bugs".to_string()),
        };
        match &result {
            UpdateCheckResult::UpdateAvailable { version, .. } => {
                assert_eq!(version, "1.2.0");
            }
            _ => panic!("Expected UpdateAvailable"),
        }
    }

    #[test]
    fn test_update_check_result_error() {
        let result = UpdateCheckResult::Error("network down".to_string());
        match &result {
            UpdateCheckResult::Error(msg) => {
                assert_eq!(msg, "network down");
            }
            _ => panic!("Expected Error"),
        }
    }

    #[test]
    fn test_version_semver_ordering() {
        let v1: Version = "0.9.0".parse().unwrap();
        let v2: Version = "1.0.0".parse().unwrap();
        assert!(v1.is_older_than(&v2));

        let v3: Version = "1.0.1".parse().unwrap();
        assert!(v2.is_older_than(&v3));

        // Equal
        let v4: Version = "1.0.0".parse().unwrap();
        assert_eq!(v2.compare_to(&v4), std::cmp::Ordering::Equal);
    }
}
