//! Finds the Chrome or Chromium binary that PDF reports run, and downloads one
//! when none is installed.
//!
//! The order is:
//! 1. The `CHROME` environment variable. When it is set and not empty, Goose
//!    uses that path and never downloads; a path that is not an executable
//!    file is an error.
//! 2. An installed browser, found by `headless_chrome` on `PATH` and in the
//!    standard install locations.
//! 3. A Chrome for Testing headless shell that Goose downloaded earlier.
//! 4. A new download of the current stable Chrome for Testing headless shell,
//!    unpacked with the system `unzip` (`tar` on Windows). Goose never
//!    downloads when built for musl, because Chrome for Testing needs glibc.
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

const INSTALL_OR_SET_CHROME: &str =
    "Install Chrome or Chromium, or set CHROME to the full path of its binary.";

/// How Goose found the Chrome binary it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChromeSource {
    /// The `CHROME` environment variable.
    Env,
    /// An installed browser.
    Installed,
    /// A Chrome for Testing download from an earlier run.
    Cached,
    /// A Chrome for Testing download made by this run.
    Downloaded,
}

impl ChromeSource {
    /// How the binary was found, for logs and error messages.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            ChromeSource::Env => "set by CHROME",
            ChromeSource::Installed => "installed",
            ChromeSource::Cached => "Chrome for Testing, fetched by an earlier run",
            ChromeSource::Downloaded => "Chrome for Testing, fetched by this run",
        }
    }
}

/// The Chrome binary PDF reports run, and how Goose found it.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedChrome {
    pub(crate) path: PathBuf,
    pub(crate) source: ChromeSource,
    /// The version, when known without running the binary.
    pub(crate) version: Option<String>,
}

/// The `CHROME` environment variable, when it is set and not empty.
pub(crate) fn chrome_env() -> Option<PathBuf> {
    std::env::var_os("CHROME")
        .filter(|chrome| !chrome.is_empty())
        .map(PathBuf::from)
}

/// Confirms `CHROME` names an executable file. The error is the message to show.
pub(crate) fn check_chrome_env(chrome: &Path) -> Result<(), String> {
    let shown = chrome.display();
    let metadata = std::fs::metadata(chrome)
        .map_err(|_| format!("CHROME is set to {shown}, which does not exist"))?;
    if metadata.is_dir() {
        return Err(match app_bundle_binary(chrome) {
            Some(binary) => format!(
                "CHROME is set to {shown}, which is an app bundle; set it to the binary inside it, {}",
                binary.display()
            ),
            None => format!("CHROME is set to {shown}, which is a directory, not a Chrome or Chromium binary"),
        });
    }
    if !metadata.is_file() {
        return Err(format!(
            "CHROME is set to {shown}, which is not a regular file"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!("CHROME is set to {shown}, which is not executable"));
        }
    }
    Ok(())
}

/// The error for a `CHROME` that [`check_chrome_env`] rejected.
pub(crate) fn chrome_env_error(value: String) -> GooseError {
    chrome_error(
        value,
        "Set CHROME to the full path of an existing Chrome or Chromium binary, or unset it so Goose finds or downloads one.",
    )
}

/// The binary inside a macOS app bundle such as `Google Chrome.app`.
fn app_bundle_binary(bundle: &Path) -> Option<PathBuf> {
    if bundle.extension()? != "app" {
        return None;
    }
    Some(
        bundle
            .join("Contents")
            .join("MacOS")
            .join(bundle.file_stem()?),
    )
}

/// Finds the Chrome or Chromium binary to run, downloading one when none is
/// installed.
pub(crate) async fn resolve_chrome() -> Result<ResolvedChrome, GooseError> {
    if let Some(chrome) = chrome_env() {
        check_chrome_env(&chrome).map_err(chrome_env_error)?;
        return Ok(ResolvedChrome {
            path: chrome,
            source: ChromeSource::Env,
            version: None,
        });
    }

    if let Ok(installed) = headless_chrome::browser::default_executable() {
        return Ok(ResolvedChrome {
            path: installed,
            source: ChromeSource::Installed,
            version: None,
        });
    }

    if cfg!(target_env = "musl") {
        return Err(chrome_error(
            "Goose found no installed Chrome or Chromium, and does not download one on musl".to_string(),
            "Chrome for Testing builds need glibc, so they can't run on a musl system such as Alpine. Install Chromium (apk add chromium), or set CHROME to the full path of a Chrome or Chromium binary.",
        ));
    }

    let platform = platform().ok_or_else(|| {
        chrome_error(
            format!(
                "no Chrome for Testing build exists for {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            INSTALL_OR_SET_CHROME,
        )
    })?;
    let cache = cache_dir().ok_or_else(|| {
        chrome_error(
            "no user cache directory to download Chrome into".to_string(),
            INSTALL_OR_SET_CHROME,
        )
    })?;

    if let Some((version, binary)) = cached_chrome(&cache, platform) {
        return Ok(ResolvedChrome {
            path: binary,
            source: ChromeSource::Cached,
            version: Some(version),
        });
    }

    unpack_tool_available().map_err(|value| {
        chrome_error(
            value,
            "Install unzip (apt install unzip, dnf install unzip, apk add unzip or brew install unzip), or set CHROME to the full path of a Chrome or Chromium binary.",
        )
    })?;
    let (version, binary) = download_chrome(&cache, platform).await.map_err(|e| {
        chrome_error(
            format!("Goose found no installed Chrome or Chromium, and downloading one failed: {e}"),
            INSTALL_OR_SET_CHROME,
        )
    })?;
    Ok(ResolvedChrome {
        path: binary,
        source: ChromeSource::Downloaded,
        version: Some(version),
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

/// The version and binary of the newest complete download in the cache for
/// this platform, if any.
///
/// Each download is unpacked into `<version>-<platform>`; a directory without
/// the binary in it is ignored.
fn cached_chrome(cache: &Path, platform: &str) -> Option<(String, PathBuf)> {
    let suffix = format!("-{platform}");
    std::fs::read_dir(cache)
        .ok()?
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            let version = name.strip_suffix(&suffix)?.to_string();
            let key = version_key(&version)?;
            let binary = binary_in(&cache.join(&name), platform);
            binary.is_file().then_some((key, version, binary))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, version, binary)| (version, binary))
}

fn http_client(connect_timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(connect_timeout)
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())
}

/// The stable version and the download URL for `platform` from the Chrome for
/// Testing JSON.
fn parse_stable_download(json: &serde_json::Value, platform: &str) -> Option<(String, String)> {
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

/// Reads the stable version and download URL for `platform` from Chrome for Testing.
async fn stable_download(
    client: &reqwest::Client,
    platform: &str,
) -> Result<(String, String), String> {
    let json: serde_json::Value = client
        .get(CHROME_FOR_TESTING_JSON)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|e| format!("reading {CHROME_FOR_TESTING_JSON}: {e}"))?
        .json()
        .await
        .map_err(|e| format!("reading {CHROME_FOR_TESTING_JSON}: {e}"))?;
    parse_stable_download(&json, platform).ok_or_else(|| {
        format!("{CHROME_FOR_TESTING_JSON} lists no stable {DOWNLOAD_NAME} for {platform}")
    })
}

/// Downloads and unpacks the stable Chrome for Testing headless shell into the
/// cache, and returns its version and the path of its binary.
async fn download_chrome(cache: &Path, platform: &str) -> Result<(String, PathBuf), String> {
    let client = http_client(Duration::from_secs(30))?;
    let (version, url) = stable_download(&client, platform).await?;
    let binary = install(&client, cache, platform, &version, &url).await?;
    Ok((version, binary))
}

/// Downloads and unpacks one Chrome for Testing build into the cache, and
/// returns the path of its binary.
async fn install(
    client: &reqwest::Client,
    cache: &Path,
    platform: &str,
    version: &str,
    url: &str,
) -> Result<PathBuf, String> {
    let install = cache.join(format!("{version}-{platform}"));
    // Unpack into a directory of our own, then rename it into place, so a
    // failed or concurrent download never leaves a partial install behind.
    let partial = cache.join(format!(".{version}-{platform}.{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&partial).await;
    tokio::fs::create_dir_all(&partial)
        .await
        .map_err(|e| format!("creating {}: {e}", partial.display()))?;

    let result = download_and_unpack(client, url, &partial, &install, platform, version).await;
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

    Ok(binary_in(&install, platform))
}

async fn download_and_unpack(
    client: &reqwest::Client,
    url: &str,
    dir: &Path,
    install: &Path,
    platform: &str,
    version: &str,
) -> Result<(), String> {
    let archive = dir.join(format!("{DOWNLOAD_NAME}-{platform}.zip"));
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|e| format!("downloading {url}: {e}"))?;
    let size = response
        .content_length()
        .map(|bytes| format!(" ({} MB)", bytes.div_ceil(1_000_000)))
        .unwrap_or_default();
    info!(
        "downloading Chrome for Testing {version}{size} from {url} to {}, before the load test starts",
        install.display()
    );
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

/// The system tool that unpacks the download: `unzip`, or `tar` on Windows,
/// which has shipped a `tar` that reads zip archives since Windows 10.
fn unpack_tool() -> &'static str {
    if cfg!(target_os = "windows") {
        "tar"
    } else {
        "unzip"
    }
}

/// Confirms the unpack tool runs, before anything is downloaded.
fn unpack_tool_available() -> Result<(), String> {
    let tool = unpack_tool();
    let flag = if tool == "tar" { "--version" } else { "-v" };
    match std::process::Command::new(tool)
        .arg(flag)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "Goose needs {tool} to unpack the Chrome for Testing download, and `{tool} {flag}` failed ({status})"
        )),
        Err(e) => Err(format!(
            "Goose needs {tool} to unpack the Chrome for Testing download, and could not run it: {e}"
        )),
    }
}

/// Unpacks a zip archive with [`unpack_tool`].
fn unpack(archive: &Path, dir: &Path) -> Result<(), String> {
    let tool = unpack_tool();
    let output = if tool == "tar" {
        std::process::Command::new(tool)
            .arg("-xf")
            .arg(archive)
            .arg("-C")
            .arg(dir)
            .output()
    } else {
        std::process::Command::new(tool)
            .arg("-q")
            .arg(archive)
            .arg("-d")
            .arg(dir)
            .output()
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

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("goose-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

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
            parse_stable_download(&json, "linux64"),
            Some((
                "155.0.8059.84".to_string(),
                "https://example.com/shell-linux64.zip".to_string()
            ))
        );
        assert_eq!(parse_stable_download(&json, "win64"), None);
    }

    #[test]
    fn cached_chrome_picks_the_newest_complete_download() {
        let cache = temp_dir("chrome-cache");
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
            Some((
                "10.0.0.2".to_string(),
                binary_in(&cache.join("10.0.0.2-linux64"), platform)
            ))
        );
        assert_eq!(cached_chrome(&cache, "win64"), None);
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn app_bundles_name_their_binary() {
        assert_eq!(
            app_bundle_binary(Path::new("/Applications/Google Chrome.app")),
            Some(PathBuf::from(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
            ))
        );
        assert_eq!(
            app_bundle_binary(Path::new("/Applications/Chromium.app")),
            Some(PathBuf::from(
                "/Applications/Chromium.app/Contents/MacOS/Chromium"
            ))
        );
        assert_eq!(app_bundle_binary(Path::new("/Applications")), None);
    }

    #[test]
    fn check_chrome_env_rejects_what_cannot_run() {
        let dir = temp_dir("chrome-env");

        let missing = dir.join("missing");
        assert!(check_chrome_env(&missing)
            .unwrap_err()
            .contains("does not exist"));

        assert!(check_chrome_env(&dir).unwrap_err().contains("directory"));

        let bundle = dir.join("Google Chrome.app");
        std::fs::create_dir_all(&bundle).unwrap();
        let error = check_chrome_env(&bundle).unwrap_err();
        assert!(error.contains("Contents/MacOS/Google Chrome"), "{}", error);

        let binary = dir.join("chrome");
        std::fs::write(&binary, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(check_chrome_env(&binary)
                .unwrap_err()
                .contains("not executable"));
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(check_chrome_env(&binary), Ok(()));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Downloads Chrome for Testing into a temporary cache. Needs the network,
    /// so it is ignored by default: `cargo test --features pdf-reports --lib
    /// report::chrome -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn downloads_chrome_for_testing() {
        let cache = temp_dir("chrome-download");
        let platform = platform().unwrap();
        unpack_tool_available().unwrap();
        let (version, binary) = download_chrome(&cache, platform).await.unwrap();
        assert!(binary.is_file());
        assert_eq!(cached_chrome(&cache, platform), Some((version, binary)));
        std::fs::remove_dir_all(&cache).unwrap();
    }
}
