use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct Settings {
    pub rows: usize,
    pub columns: usize,
    pub mines: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rows: 10,
            columns: 10,
            mines: 15,
        }
    }
}

impl Settings {
    pub fn validate(self) -> Result<Self, String> {
        if !(5..=24).contains(&self.rows)
            || !(5..=30).contains(&self.columns)
            || self.mines == 0
            || self.mines >= self.rows * self.columns
        {
            return Err("Invalid custom settings: expected 5-24 rows, 5-30 columns, and 1 to fields-1 makibishi.".into());
        }
        Ok(self)
    }
}

pub fn path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is unavailable; cannot save custom settings.")?;
    #[cfg(not(target_os = "windows"))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("Cannot locate the configuration directory for custom settings.")?;
    Ok(root.join("minesweeper-bevy").join("custom.json"))
}

pub fn load(path: &Path) -> Result<Settings, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default());
        }
        Err(error) => return Err(format!("Cannot read custom settings: {error}")),
    };
    serde_json::from_slice::<Settings>(&bytes)
        .map_err(|error| format!("Cannot parse custom settings: {error}"))?
        .validate()
}

pub fn save(path: &Path, settings: Settings) -> Result<(), String> {
    settings.validate()?;
    let bytes = serde_json::to_vec(&settings)
        .map_err(|error| format!("Cannot encode custom settings: {error}"))?;
    let parent = path.parent().ok_or("Custom settings path has no parent.")?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Cannot create custom settings directory: {error}"))?;
    std::fs::write(path, bytes).map_err(|error| format!("Cannot save custom settings: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_settings_persist_and_reset() {
        let directory = std::env::temp_dir().join(format!(
            "minesweeper-custom-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let path = directory.join("custom.json");
        assert_eq!(load(&path).unwrap(), Settings::default());
        let settings = Settings {
            rows: 24,
            columns: 30,
            mines: 100,
        };
        save(&path, settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        save(&path, Settings::default()).unwrap();
        assert_eq!(load(&path).unwrap(), Settings::default());
        std::fs::write(&path, b"invalid json").unwrap();
        assert!(load(&path).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn invalid_custom_dimensions_and_mines_are_rejected() {
        for settings in [
            Settings {
                rows: 0,
                ..Settings::default()
            },
            Settings {
                columns: 31,
                ..Settings::default()
            },
            Settings {
                mines: 0,
                ..Settings::default()
            },
            Settings {
                mines: 100,
                ..Settings::default()
            },
            Settings {
                rows: usize::MAX,
                ..Settings::default()
            },
        ] {
            assert!(settings.validate().is_err());
        }
    }
}
