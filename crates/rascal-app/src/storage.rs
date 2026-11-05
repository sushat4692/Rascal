//! Small JSON state file: recent repositories, last repository and theme.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppState {
    #[serde(default)]
    pub recent: Vec<RecentRepo>,
    #[serde(default)]
    pub last_repo: Option<PathBuf>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub diff_theme: Option<String>,
    /// Repositories that were open as tabs when the session was last saved.
    /// `None` means no session was saved yet (older state files).
    #[serde(default)]
    pub open_repos: Option<Vec<PathBuf>>,
    #[serde(default)]
    pub active_repo: Option<PathBuf>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub ui_font: Option<String>,
    #[serde(default)]
    pub mono_font: Option<String>,
    #[serde(default)]
    pub diff_layout: Option<String>,
    #[serde(default)]
    pub image_diff_mode: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentRepo {
    pub path: PathBuf,
    pub name: String,
    #[serde(default)]
    pub last_opened: i64,
}

#[derive(Clone, Debug)]
pub struct Storage {
    path: Option<PathBuf>,
}

impl Storage {
    pub fn new() -> Self {
        Self {
            path: default_state_path(),
        }
    }

    #[cfg(test)]
    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub fn load(&self) -> AppState {
        let Some(path) = &self.path else {
            return AppState::default();
        };
        let Ok(contents) = std::fs::read_to_string(path) else {
            return AppState::default();
        };
        serde_json::from_str(&contents).unwrap_or_default()
    }

    pub fn save(&self, state: &AppState) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        let contents = serde_json::to_vec_pretty(state)?;
        std::fs::write(&temp, contents)?;
        match std::fs::rename(&temp, path) {
            Ok(()) => Ok(()),
            Err(err) => {
                let _ = std::fs::remove_file(&temp);
                Err(err)
            }
        }
    }

    pub fn remember(&self, state: &mut AppState, path: &Path, name: &str) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        state.recent.retain(|entry| entry.path != canonical);
        state.recent.insert(
            0,
            RecentRepo {
                path: canonical.clone(),
                name: name.to_string(),
                last_opened: chrono::Local::now().timestamp(),
            },
        );
        state.recent.truncate(20);
        state.last_repo = Some(canonical);
        let _ = self.save(state);
    }

    pub fn set_theme(&self, state: &mut AppState, theme: &str) {
        state.theme = Some(theme.to_string());
        let _ = self.save(state);
    }
}

fn default_state_path() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Rascal").join("state.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_state_and_dedupes_recent() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::at(dir.path().join("state.json"));
        let mut state = AppState::default();

        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        storage.remember(&mut state, &repo, "repo");
        storage.remember(&mut state, &repo, "repo");
        assert_eq!(state.recent.len(), 1);

        let loaded = storage.load();
        assert_eq!(loaded.recent.len(), 1);
        assert_eq!(
            loaded.last_repo,
            Some(std::fs::canonicalize(&repo).unwrap())
        );
    }

    #[test]
    fn corrupt_state_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "not json").unwrap();
        let storage = Storage::at(path);
        assert!(storage.load().recent.is_empty());
    }
}
