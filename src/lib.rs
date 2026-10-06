/*
 *    This file is a part of the Koshka shell, (c) toiletbril, 2026
 *    See the top-level LICENSE file for the licensing information.
 *
 * This file starts the Koshka language server for Zed. It resolves the kosh
 * binary from the binary.path setting, then PATH, then the newest
 * fennec-support/kosh release, and appends --as-language-server when the
 * configured arguments lack it. It compares the version of the binary with the
 * newest release once per session and updates only its own download.
 */

use std::cmp::Ordering;

use zed_extension_api::{
    self as zed, process, settings::LspSettings, Architecture, Command, DownloadedFileType,
    GithubReleaseOptions, LanguageServerId, LanguageServerInstallationStatus, Os, Result, Worktree,
};

const BINARY_NAME: &str = "kosh";
const SERVER_ARGUMENT: &str = "--as-language-server";
const RELEASE_REPOSITORY: &str = "fennec-support/kosh";

struct LatestRelease {
    version: String,
    download_url: Option<String>,
}

struct KoshExtension {
    downloaded_binary_path: Option<String>,
    latest_release: Option<Result<LatestRelease>>,
}

struct Version {
    numbers: Vec<u64>,
    is_prerelease: bool,
}

/*
 * An asset name holds the platform and the processor. An arm64 asset names
 * the processor aarch64, and an x86-64 asset names it amd64. No asset is built
 * for a 32-bit machine.
 */
fn get_asset_name_prefixes() -> Vec<String> {
    let (os, architecture) = zed::current_platform();

    let platform_name = match os {
        Os::Mac => "darwin",
        Os::Linux => "linux",
        Os::Windows => "win32",
    };

    let architecture_names: &[&str] = match architecture {
        Architecture::Aarch64 => &["aarch64", "arm64"],
        Architecture::X8664 => &["amd64", "x86_64"],
        Architecture::X86 => &[],
    };

    architecture_names
        .iter()
        .map(|name| format!("{BINARY_NAME}-{platform_name}-{name}-"))
        .collect()
}

fn get_binary_file_name() -> &'static str {
    match zed::current_platform().0 {
        Os::Windows => "kosh.exe",
        _ => BINARY_NAME,
    }
}

/*
 * Each download lives in a directory named after its release. Earlier ones are
 * removed once the new download is in place.
 */
fn remove_earlier_downloads(current_directory: &str) {
    let entries = match std::fs::read_dir(".") {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_earlier_download =
            name.starts_with(&format!("{BINARY_NAME}-")) && name != current_directory;

        if is_earlier_download {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/*
 * A release tag or the first line of the version output is read as dotted
 * numbers. A suffix after a slash is dropped, and a suffix after a hyphen marks
 * a prerelease.
 */
fn parse_version(text: &str) -> Option<Version> {
    let token = text.trim().trim_start_matches('v');
    let token = token.split('/').next()?;
    let (core, suffix) = match token.split_once('-') {
        Some((core, suffix)) => (core, Some(suffix)),
        None => (token, None),
    };

    let numbers = core
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<u64>>>()?;

    Some(Version {
        numbers,
        is_prerelease: suffix.is_some(),
    })
}

fn compare_versions(left: &Version, right: &Version) -> Ordering {
    let length = left.numbers.len().max(right.numbers.len());

    for index in 0..length {
        let left_number = left.numbers.get(index).copied().unwrap_or(0);
        let right_number = right.numbers.get(index).copied().unwrap_or(0);

        match left_number.cmp(&right_number) {
            Ordering::Equal => {}
            other => return other,
        }
    }

    right.is_prerelease.cmp(&left.is_prerelease)
}

fn read_binary_version(binary_path: &str) -> Option<Version> {
    let output = process::Command::new(binary_path)
        .arg("--version")
        .output()
        .ok()?;

    if output.status != Some(0) {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let first_line = text.lines().next()?;

    parse_version(first_line.split_whitespace().last()?)
}

fn is_older_than_release(binary_path: &str, release: &LatestRelease) -> Option<bool> {
    let installed = read_binary_version(binary_path)?;
    let latest = parse_version(&release.version)?;

    Some(compare_versions(&installed, &latest) == Ordering::Less)
}

fn find_stored_binary() -> Option<String> {
    let entries = std::fs::read_dir(".").ok()?;
    let mut newest: Option<(Option<Version>, String)> = None;

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let tag = match name.strip_prefix(&format!("{BINARY_NAME}-")) {
            Some(tag) => tag.to_string(),
            None => continue,
        };

        let binary_path = format!("{name}/{}", get_binary_file_name());

        if std::fs::metadata(&binary_path).is_err() {
            continue;
        }

        let version = parse_version(&tag);
        let is_newer = match (&version, &newest) {
            (_, None) => true,
            (Some(candidate), Some((Some(best), _))) => {
                compare_versions(candidate, best) == Ordering::Greater
            }
            (Some(_), Some((None, _))) => true,
            (None, Some(_)) => false,
        };

        if is_newer {
            newest = Some((version, binary_path));
        }
    }

    newest.map(|(_, binary_path)| binary_path)
}

impl KoshExtension {
    /*
     * The newest release is read once per session, and a failure is kept too.
     */
    fn get_latest_release(&mut self) -> &Result<LatestRelease> {
        self.latest_release.get_or_insert_with(|| {
            let release = zed::latest_github_release(
                RELEASE_REPOSITORY,
                GithubReleaseOptions {
                    require_assets: true,
                    pre_release: false,
                },
            )?;

            let download_url = get_asset_name_prefixes().iter().find_map(|prefix| {
                release
                    .assets
                    .iter()
                    .find(|asset| asset.name.starts_with(prefix))
                    .map(|asset| asset.download_url.clone())
            });

            Ok(LatestRelease {
                version: release.version,
                download_url,
            })
        })
    }

    /*
     * A binary from the settings or PATH is never replaced. An older one is
     * reported in the log, and a failed lookup or probe is ignored.
     */
    fn report_outdated_binary(&mut self, binary_path: &str) {
        let release = match self.get_latest_release() {
            Ok(release) => release,
            Err(_) => return,
        };

        if is_older_than_release(binary_path, release) == Some(true) {
            eprintln!(
                "{BINARY_NAME} at {binary_path} is older than the latest release {}. \
                 Update it to get the newest language server.",
                release.version
            );
        }
    }

    /*
     * The stored shell is updated from the newest release when it is older.
     * Prereleases are skipped. A failed lookup or probe keeps the stored shell.
     */
    fn download_binary(&mut self, language_server_id: &LanguageServerId) -> Result<String> {
        if let Some(path) = &self.downloaded_binary_path {
            if std::fs::metadata(path).is_ok() {
                return Ok(path.clone());
            }
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let stored_path = find_stored_binary();

        let release = match self.get_latest_release() {
            Ok(release) => release,
            Err(error) => {
                return match stored_path {
                    Some(path) => {
                        zed::set_language_server_installation_status(
                            language_server_id,
                            &LanguageServerInstallationStatus::None,
                        );
                        self.downloaded_binary_path = Some(path.clone());

                        Ok(path)
                    }
                    None => Err(error.clone()),
                };
            }
        };

        let version_directory = format!("{BINARY_NAME}-{}", release.version);
        let binary_path = format!("{version_directory}/{}", get_binary_file_name());

        let should_download = match &stored_path {
            None => true,
            Some(path) => {
                std::fs::metadata(&binary_path).is_err()
                    && is_older_than_release(path, release) == Some(true)
            }
        };

        if !should_download {
            let path = stored_path.unwrap_or(binary_path);

            zed::set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::None,
            );
            self.downloaded_binary_path = Some(path.clone());

            return Ok(path);
        }

        let download_url = release.download_url.clone().ok_or_else(|| {
            format!(
                "Release {} has no {BINARY_NAME} binary for this platform. \
                 Build the shell from source and put it on your PATH.",
                release.version
            )
        })?;

        if std::fs::metadata(&binary_path).is_err() {
            zed::set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );

            std::fs::create_dir_all(&version_directory)
                .map_err(|error| format!("The download directory was not created. {error}"))?;

            zed::download_file(
                &download_url,
                &binary_path,
                DownloadedFileType::Uncompressed,
            )?;
            zed::make_file_executable(&binary_path)?;

            remove_earlier_downloads(&version_directory);
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::None,
        );

        self.downloaded_binary_path = Some(binary_path.clone());

        Ok(binary_path)
    }
}

impl zed::Extension for KoshExtension {
    fn new() -> Self {
        Self {
            downloaded_binary_path: None,
            latest_release: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Command> {
        let binary_settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.binary);

        let configured_path = binary_settings
            .as_ref()
            .and_then(|binary| binary.path.clone());

        let command = match configured_path {
            Some(path) => {
                self.report_outdated_binary(&path);

                path
            }
            None => match worktree.which(BINARY_NAME) {
                Some(path) => {
                    self.report_outdated_binary(&path);

                    path
                }
                None => self.download_binary(language_server_id).map_err(|error| {
                    zed::set_language_server_installation_status(
                        language_server_id,
                        &LanguageServerInstallationStatus::Failed(error.clone()),
                    );

                    format!(
                        "{BINARY_NAME} was not found on the PATH, and the release download \
                         failed. {error}"
                    )
                })?,
            },
        };

        let mut args = binary_settings
            .and_then(|binary| binary.arguments)
            .unwrap_or_default();

        if !args.iter().any(|argument| argument == SERVER_ARGUMENT) {
            args.push(SERVER_ARGUMENT.to_string());
        }

        Ok(Command {
            command,
            args,
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(KoshExtension);
