use std::{
    ffi::OsString,
    io,
    path::PathBuf,
    process::{Command, Stdio},
};

/// A zoxide directory with its ranking score.
#[derive(Clone, Debug)]
pub struct ZoxideEntry {
    /// The directory recorded by zoxide.
    pub path: PathBuf,
    /// zoxide's score for the directory.
    pub score: f64,
}

/// zoxide candidates and any non-fatal source warning.
#[derive(Debug, Default)]
pub struct ZoxideSource {
    /// Parsed zoxide records.
    pub entries: Vec<ZoxideEntry>,
    /// A user-facing reason that no zoxide records are available.
    pub warning: Option<String>,
}

const SYSTEM_ZOXIDE_BINARIES: &[&str] = &[
    "/opt/homebrew/bin/zoxide",
    "/usr/local/bin/zoxide",
    "/opt/local/bin/zoxide",
    "/home/linuxbrew/.linuxbrew/bin/zoxide",
    "/run/current-system/sw/bin/zoxide",
    "/snap/bin/zoxide",
    "/usr/bin/zoxide",
    "/bin/zoxide",
];

/// Loads zoxide records from PATH and standard install locations without failing when unavailable.
pub fn load() -> ZoxideSource {
    load_from_paths(zoxide_binaries())
}

fn zoxide_binaries() -> Vec<PathBuf> {
    zoxide_binaries_from_environment(
        std::env::var_os("HERDR_WORKSPACER_ZOXIDE_PATH"),
        std::env::var_os("HOME"),
        std::env::var_os("CARGO_HOME"),
        std::env::var_os("CONDA_PREFIX"),
    )
}

fn zoxide_binaries_from_environment(
    configured_path: Option<OsString>,
    home: Option<OsString>,
    cargo_home: Option<OsString>,
    conda_prefix: Option<OsString>,
) -> Vec<PathBuf> {
    let home = home.map(PathBuf::from);
    let cargo_home = cargo_home.map(PathBuf::from);
    let mut binaries = Vec::with_capacity(14);

    if let Some(path) = configured_path {
        binaries.push(PathBuf::from(path));
    }
    binaries.push(PathBuf::from("zoxide"));

    if let Some(home) = home {
        binaries.push(home.join(".local/bin/zoxide"));
        let cargo_home = cargo_home.unwrap_or_else(|| home.join(".cargo"));
        binaries.push(cargo_home.join("bin/zoxide"));
        binaries.push(home.join(".nix-profile/bin/zoxide"));
        binaries.push(home.join(".linuxbrew/bin/zoxide"));
        binaries.push(home.join(".config/guix/current/bin/zoxide"));
    } else if let Some(cargo_home) = cargo_home {
        binaries.push(cargo_home.join("bin/zoxide"));
    }

    if let Some(conda_prefix) = conda_prefix {
        binaries.push(PathBuf::from(conda_prefix).join("bin/zoxide"));
    }
    binaries.extend(SYSTEM_ZOXIDE_BINARIES.iter().map(PathBuf::from));
    binaries
}

fn load_from_paths(paths: impl IntoIterator<Item = PathBuf>) -> ZoxideSource {
    for binary in paths {
        let output = Command::new(&binary)
            .args(["query", "-ls"])
            .stdin(Stdio::null())
            .output();

        match output {
            Ok(output) if output.status.success() => {
                return ZoxideSource {
                    entries: String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .filter_map(parse_line)
                        .collect(),
                    warning: None,
                };
            }
            Ok(_) => {
                return ZoxideSource {
                    entries: Vec::new(),
                    warning: Some("zoxide query failed. Showing open workspaces only.".to_string()),
                };
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {
                return ZoxideSource {
                    entries: Vec::new(),
                    warning: Some(
                        "zoxide could not run. Showing open workspaces only.".to_string(),
                    ),
                };
            }
        }
    }

    ZoxideSource {
        entries: Vec::new(),
        warning: Some("zoxide not found. Showing open workspaces only.".to_string()),
    }
}

fn parse_line(line: &str) -> Option<ZoxideEntry> {
    let line = line.trim_start_matches(char::is_whitespace);
    let (score, path) = line
        .split_once('\t')
        .or_else(|| line.split_once(char::is_whitespace))?;
    let score = score.trim().parse::<f64>().ok()?;
    let path = path.trim_start_matches(char::is_whitespace);

    (score.is_finite() && !path.is_empty()).then(|| ZoxideEntry {
        path: PathBuf::from(path),
        score,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[cfg(unix)]
    static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[cfg(unix)]
    struct TemporaryDirectory {
        path: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl TemporaryDirectory {
        fn new() -> anyhow::Result<Self> {
            let sequence = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos());
            let path = std::env::temp_dir().join(format!(
                "herdr-workspacer-zoxide-{}-{timestamp}-{sequence}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path)?;
            Ok(Self { path })
        }
    }

    #[cfg(unix)]
    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn parses_tab_delimited_paths_with_spaces() {
        let entry = parse_line("42.5\t/Users/example/with spaces");

        assert_eq!(
            entry.map(|entry| (entry.score, entry.path)),
            Some((42.5, PathBuf::from("/Users/example/with spaces")))
        );
    }

    #[test]
    fn parses_space_padded_zoxide_scores() {
        let entry = parse_line(" 214.0 /Users/example/project");

        assert_eq!(
            entry.map(|entry| (entry.score, entry.path)),
            Some((214.0, PathBuf::from("/Users/example/project")))
        );
    }

    #[test]
    fn skips_malformed_lines() {
        assert!(parse_line("not a zoxide record").is_none());
        assert!(parse_line("NaN\t/path").is_none());
        assert!(parse_line("1.0\t").is_none());
    }

    #[test]
    fn discovers_user_and_package_manager_install_locations() -> anyhow::Result<()> {
        let binaries = zoxide_binaries_from_environment(
            Some(std::ffi::OsString::from("/custom/zoxide")),
            Some(std::ffi::OsString::from("/home/example")),
            Some(std::ffi::OsString::from("/cargo")),
            Some(std::ffi::OsString::from("/conda")),
        );
        let expected = [
            "/custom/zoxide",
            "zoxide",
            "/home/example/.local/bin/zoxide",
            "/cargo/bin/zoxide",
            "/home/example/.nix-profile/bin/zoxide",
            "/home/example/.linuxbrew/bin/zoxide",
            "/home/example/.config/guix/current/bin/zoxide",
            "/conda/bin/zoxide",
            "/opt/homebrew/bin/zoxide",
            "/usr/local/bin/zoxide",
            "/opt/local/bin/zoxide",
            "/home/linuxbrew/.linuxbrew/bin/zoxide",
            "/run/current-system/sw/bin/zoxide",
            "/snap/bin/zoxide",
            "/usr/bin/zoxide",
            "/bin/zoxide",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();

        anyhow::ensure!(
            binaries == expected,
            "zoxide discovery returned unexpected locations"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn tries_fallback_binaries_after_path_lookup_fails() -> anyhow::Result<()> {
        let directory = TemporaryDirectory::new()?;
        let binary = directory.path.join("zoxide");
        fs::write(&binary, "#!/bin/sh\nprintf '12\\t/project\\n'\n")?;
        let mut permissions = fs::metadata(&binary)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&binary, permissions)?;

        let source = load_from_paths([PathBuf::from("herdr-workspacer-missing-zoxide"), binary]);

        anyhow::ensure!(
            source.warning.is_none(),
            "fallback binary produced a warning"
        );
        anyhow::ensure!(
            source.entries.len() == 1,
            "fallback binary produced no entry"
        );
        let Some(entry) = source.entries.first() else {
            anyhow::bail!("fallback binary produced no first entry");
        };
        anyhow::ensure!(
            entry.path.as_path() == std::path::Path::new("/project")
                && (entry.score - 12.0).abs() < f64::EPSILON,
            "fallback binary produced an unexpected entry"
        );
        Ok(())
    }
}
