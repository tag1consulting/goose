//! Finds the Chrome or Chromium binary that PDF reports run, and downloads one
//! when none is installed.
//!
//! The order is:
//! 1. The `CHROME` environment variable. When it is set and not empty, Goose
//!    uses that path and never downloads; a path that does not exist is an error.
//! 2. An installed browser, found by `headless_chrome` on `PATH` and in the
//!    standard install locations.
//! 3. A Chrome for Testing headless shell that Goose downloaded earlier.
//! 4. A new download of the current stable Chrome for Testing headless shell,
//!    unpacked with the system `unzip` (`tar` on Windows).
//!
//! Downloads go to `goose/chrome` under the user cache directory:
//! `$XDG_CACHE_HOME` or `~/.cache` on Linux, `~/Library/Caches` on macOS and
//! `%LOCALAPPDATA%` on Windows.
//!
//! **Note**: This module is only available when compiled with the `pdf-reports` feature flag.

use crate::GooseError;
use log::info;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Lists the Chrome for Testing downloads of each release channel.
const CHROME_FOR_TESTING_JSON: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";

/// The Chrome for Testing build Goose downloads: the headless shell, which is
/// smaller than full Chrome and all that PDF generation needs.
const DOWNLOAD_NAME: &str = "chrome-headless-shell";

/// Returns the path of the Chrome or Chromium binary to run, downloading one
/// when none is installed.
pub(crate) async fn find_or_download_chrome() -> Result<PathBuf, GooseError> {
    if let Some(chrome) = std::env::var_os("CHROME").filter(|chrome| !chrome.is_empty()) {
        let chrome = PathBuf::from(chrome);
        if chrome.exists() {
            return Ok(chrome);
        }
        return Err(chrome_error(
            format!("CHROME is set to {}, which does not exist", chrome.display()),
            "Set CHROME to the full path of an existing Chrome or Chromium binary, or unset it so Goose finds or downloads one.",
        ));
    }

    if let Ok(installed) = headless_chrome::browser::default_executable() {
        return Ok(installed);
    }

    let platform = platform().ok_or_else(|| {
        chrome_error(
            format!(
                "no Chrome for Testing build exists for {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            "Install Chrome or Chromium, or set CHROME to the full path of its binary.",
        )
    })?;
    let cache = cache_dir().ok_or_else(|| {
        chrome_error(
            "no user cache directory to download Chrome into".to_string(),
            "Install Chrome or Chromium, or set CHROME to the full path of its binary.",
        )
    })?;

    if let Some(cached) = cached_chrome(&cache, platform) {
        return Ok(cached);
    }

    download_chrome(&cache, platform).await.map_err(|e| {
        chrome_error(
            format!("Goose found no installed Chrome or Chromium, and downloading one failed: {e}"),
            "Install Chrome or Chromium, or set CHROME to the full path of its binary.",
        )
    })
}

fn chrome_error(value: String, detail: &str) -> GooseError {
    GooseError::InvalidOption {
        option: "--pdf".to_string(),
        value,
        detail: detail.to_string(),
    }
}

/// The Chrome for Testing platform name for this build of Goose.
fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("macos", "x86_64") => Some("mac-x64"),
        ("macos", "aarch64") => Some("mac-arm64"),
        ("windows", "x86_64") => Some("win64"),
        ("windows", "x86") => Some("win32"),
        _ => None,
    }
}

/// The directory Goose downloads Chrome into.
fn cache_dir() -> Option<PathBuf> {
    let non_empty = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let base = if cfg!(target_os = "windows") {
        PathBuf::from(non_empty("LOCALAPPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(non_empty("HOME")?)
            .join("Library")
            .join("Caches")
    } else {
        match non_empty("XDG_CACHE_HOME") {
            Some(cache) => PathBuf::from(cache),
            None => PathBuf::from(non_empty("HOME")?).join(".cache"),
        }
    };
    Some(base.join("goose").join("chrome"))
}

/// The binary inside an unpacked Chrome for Testing download.
fn binary_in(install: &Path, platform: &str) -> PathBuf {
    let binary = if platform.starts_with("win") {
        format!("{DOWNLOAD_NAME}.exe")
    } else {
        DOWNLOAD_NAME.to_string()
    };
    install
        .join(format!("{DOWNLOAD_NAME}-{platform}"))
        .join(binary)
}

/// Splits a Chrome version such as `155.0.8059.84` into numbers for ordering.
fn version_key(version: &str) -> Option<Vec<u32>> {
    version.split('.').map(|part| part.parse().ok()).collect()
}

/// The newest complete download in the cache for this platform, if any.
///
/// Each download is unpacked into `<version>-<platform>`; a directory without
/// the binary in it is ignored.
fn cached_chrome(cache: &Path, platform: &str) -> Option<PathBuf> {
    let suffix = format!("-{platform}");
    std::fs::read_dir(cache)
        .ok()?
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            let version = version_key(name.strip_suffix(&suffix)?)?;
            let binary = binary_in(&cache.join(&name), platform);
            binary.is_file().then_some((version, binary))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, binary)| binary)
}

/// The stable version and the download URL for `platform` from the Chrome for
/// Testing JSON.
fn stable_download(json: &serde_json::Value, platform: &str) -> Option<(String, String)> {
    let stable = json.get("channels")?.get("Stable")?;
    let version = stable.get("version")?.as_str()?;
    let url = stable
        .get("downloads")?
        .get(DOWNLOAD_NAME)?
        .as_array()?
        .iter()
        .find(|download| download.get("platform").and_then(|p| p.as_str()) == Some(platform))?
        .get("url")?
        .as_str()?;
    Some((version.to_string(), url.to_string()))
}

/// Downloads and unpacks the stable Chrome for Testing headless shell into the
/// cache, and returns the path of its binary.
async fn download_chrome(cache: &Path, platform: &str) -> Result<PathBuf, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = client
        .get(CHROME_FOR_TESTING_JSON)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|e| format!("reading {CHROME_FOR_TESTING_JSON}: {e}"))?
        .json()
        .await
        .map_err(|e| format!("reading {CHROME_FOR_TESTING_JSON}: {e}"))?;
    let (version, url) = stable_download(&json, platform).ok_or_else(|| {
        format!("{CHROME_FOR_TESTING_JSON} lists no stable {DOWNLOAD_NAME} for {platform}")
    })?;

    let install = cache.join(format!("{version}-{platform}"));
    // Unpack into a directory of our own, then rename it into place, so a
    // failed or concurrent download never leaves a partial install behind.
    let partial = cache.join(format!(".{version}-{platform}.{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&partial).await;
    tokio::fs::create_dir_all(&partial)
        .await
        .map_err(|e| format!("creating {}: {e}", partial.display()))?;

    let result = download_and_unpack(&client, &url, &partial, platform).await;
    let result = match result {
        Ok(()) => match tokio::fs::rename(&partial, &install).await {
            Ok(()) => Ok(()),
            // Another Goose process finished the same download first.
            Err(_) if binary_in(&install, platform).is_file() => Ok(()),
            Err(e) => Err(format!("moving the download to {}: {e}", install.display())),
        },
        Err(e) => Err(e),
    };
    let _ = tokio::fs::remove_dir_all(&partial).await;
    result?;

    let binary = binary_in(&install, platform);
    info!("using Chrome for Testing {version} at {}", binary.display());
    Ok(binary)
}

async fn download_and_unpack(
    client: &reqwest::Client,
    url: &str,
    dir: &Path,
    platform: &str,
) -> Result<(), String> {
    info!("no Chrome or Chromium found, downloading {url}");
    let archive = dir.join(format!("{DOWNLOAD_NAME}-{platform}.zip"));
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|e| format!("downloading {url}: {e}"))?;
    let mut file = tokio::fs::File::create(&archive)
        .await
        .map_err(|e| format!("creating {}: {e}", archive.display()))?;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("downloading {url}: {e}"))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("writing {}: {e}", archive.display()))?;
    }
    file.flush()
        .await
        .map_err(|e| format!("writing {}: {e}", archive.display()))?;
    drop(file);

    let unpack_archive = archive.clone();
    let unpack_dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || unpack(&unpack_archive, &unpack_dir))
        .await
        .map_err(|e| e.to_string())??;
    let _ = tokio::fs::remove_file(&archive).await;

    if binary_in(dir, platform).is_file() {
        Ok(())
    } else {
        Err(format!(
            "the download did not contain {}",
            binary_in(Path::new(""), platform).display()
        ))
    }
}

/// Unpacks a zip archive with the system tool: `unzip`, or `tar` on Windows,
/// which has shipped a `tar` that reads zip archives since Windows 10.
fn unpack(archive: &Path, dir: &Path) -> Result<(), String> {
    let (tool, output) = if cfg!(target_os = "windows") {
        let output = std::process::Command::new("tar")
            .arg("-xf")
            .arg(archive)
            .arg("-C")
            .arg(dir)
            .output();
        ("tar", output)
    } else {
        let output = std::process::Command::new("unzip")
            .arg("-q")
            .arg(archive)
            .arg("-d")
            .arg(dir)
            .output();
        ("unzip", output)
    };
    let output = output.map_err(|e| format!("running {tool} to unpack the download: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{tool} could not unpack {}: {}",
            archive.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_platform_has_a_chrome_for_testing_build() {
        // Goose's CI and release targets are all Chrome for Testing platforms.
        assert!(platform().is_some());
    }

    #[test]
    fn stable_download_reads_the_platform_url() {
        let json = serde_json::json!({
            "channels": {
                "Stable": {
                    "version": "155.0.8059.84",
                    "downloads": {
                        "chrome": [
                            {"platform": "linux64", "url": "https://example.com/chrome-linux64.zip"}
                        ],
                        "chrome-headless-shell": [
                            {"platform": "linux-arm64", "url": "https://example.com/shell-linux-arm64.zip"},
                            {"platform": "linux64", "url": "https://example.com/shell-linux64.zip"}
                        ]
                    }
                }
            }
        });
        assert_eq!(
            stable_download(&json, "linux64"),
            Some((
                "155.0.8059.84".to_string(),
                "https://example.com/shell-linux64.zip".to_string()
            ))
        );
        assert_eq!(stable_download(&json, "win64"), None);
    }

    #[test]
    fn cached_chrome_picks_the_newest_complete_download() {
        let cache = std::env::temp_dir().join(format!("goose-chrome-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        let platform = "linux64";
        for version in ["9.0.0.1", "10.0.0.2", "11.0.0.3"] {
            let binary = binary_in(&cache.join(format!("{version}-{platform}")), platform);
            std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
            // 11.0.0.3 is a partial download with no binary in it.
            if version != "11.0.0.3" {
                std::fs::write(&binary, b"").unwrap();
            }
        }
        // A download for another platform and a partial download in progress.
        std::fs::create_dir_all(cache.join("12.0.0.4-mac-arm64")).unwrap();
        std::fs::create_dir_all(cache.join(".12.0.0.4-linux64.1")).unwrap();

        assert_eq!(
            cached_chrome(&cache, platform),
            Some(binary_in(&cache.join("10.0.0.2-linux64"), platform))
        );
        assert_eq!(cached_chrome(&cache, "win64"), None);
        std::fs::remove_dir_all(&cache).unwrap();
    }

    /// Downloads Chrome for Testing into a temporary cache. Needs the network,
    /// so it is ignored by default: `cargo test --features pdf-reports --lib
    /// report::chrome -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn downloads_chrome_for_testing() {
        let cache =
            std::env::temp_dir().join(format!("goose-chrome-download-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        let platform = platform().unwrap();
        let binary = download_chrome(&cache, platform).await.unwrap();
        assert!(binary.is_file());
        assert_eq!(cached_chrome(&cache, platform), Some(binary));
        std::fs::remove_dir_all(&cache).unwrap();
    }
}
