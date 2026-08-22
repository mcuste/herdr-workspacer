use std::{
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

/// Loads zoxide records without failing when zoxide is unavailable.
pub fn load() -> ZoxideSource {
    let output = Command::new("zoxide")
        .args(["query", "-ls"])
        .stdin(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() => ZoxideSource {
            entries: String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(parse_line)
                .collect(),
            warning: None,
        },
        Ok(_) => ZoxideSource {
            entries: Vec::new(),
            warning: Some("zoxide query failed. Showing open workspaces only.".to_string()),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => ZoxideSource {
            entries: Vec::new(),
            warning: Some("zoxide not found. Showing open workspaces only.".to_string()),
        },
        Err(_) => ZoxideSource {
            entries: Vec::new(),
            warning: Some("zoxide could not run. Showing open workspaces only.".to_string()),
        },
    }
}

fn parse_line(line: &str) -> Option<ZoxideEntry> {
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
    use super::*;

    #[test]
    fn parses_tab_delimited_paths_with_spaces() {
        let entry = parse_line("42.5\t/Users/example/with spaces");

        assert_eq!(
            entry.map(|entry| (entry.score, entry.path)),
            Some((42.5, PathBuf::from("/Users/example/with spaces")))
        );
    }

    #[test]
    fn skips_malformed_lines() {
        assert!(parse_line("not a zoxide record").is_none());
        assert!(parse_line("NaN\t/path").is_none());
        assert!(parse_line("1.0\t").is_none());
    }
}
