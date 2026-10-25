//! The app's own settings, in `~/.config/amgi/config.toml`. Unlike `amgi.toml`, which
//! belongs to a collection, these follow the person whichever folder is open.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use crate::AmgiError;

/// Keys the study pane already uses, which an answer can't take.
const RESERVED: [(&str, &str); 2] = [(" ", "showing the answer"), ("r", "replaying sounds")];

#[derive(Debug, Clone, PartialEq, Default, Deserialize, uniffi::Record)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub keys: AnswerKeys,
}

/// The key that rates a shown answer with each grade, one character each.
#[derive(Debug, Clone, PartialEq, Deserialize, uniffi::Record)]
#[serde(default, deny_unknown_fields)]
pub struct AnswerKeys {
    pub again: String,
    pub hard: String,
    pub good: String,
    pub easy: String,
}

impl Default for AnswerKeys {
    fn default() -> Self {
        Self { again: "1".into(), hard: "2".into(), good: "3".into(), easy: "4".into() }
    }
}

/// `$XDG_CONFIG_HOME/amgi/config.toml`, or `~/.config/amgi/config.toml`.
#[uniffi::export]
pub fn config_path() -> String {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_default();
    base.join("amgi").join("config.toml").to_string_lossy().into_owned()
}

/// The settings in `config_path()`, or the defaults when there is no file. Errors are a
/// sentence naming the file and what's wrong in it.
#[uniffi::export]
pub fn load_config() -> Result<Config, AmgiError> {
    let path = config_path();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(AmgiError::Config { message: format!("{path}: {e}") }),
    };
    parse(&text).map_err(|message| AmgiError::Config { message: format!("{path}: {message}") })
}

fn parse(text: &str) -> Result<Config, String> {
    let mut config: Config = toml::from_str(text).map_err(|e| e.to_string().trim_end().to_string())?;
    config.keys.check()?;
    Ok(config)
}

impl AnswerKeys {
    fn grades(&mut self) -> [(&'static str, &mut String); 4] {
        [("again", &mut self.again), ("hard", &mut self.hard), ("good", &mut self.good), ("easy", &mut self.easy)]
    }

    /// Lowercases each key, as Shift isn't needed to press a letter, and refuses keys
    /// that aren't one character, that another action has, or that two grades share.
    fn check(&mut self) -> Result<(), String> {
        let mut seen: Vec<(&str, String)> = Vec::new();
        for (grade, key) in self.grades() {
            *key = key.to_lowercase();
            if key.chars().count() != 1 {
                return Err(format!("keys.{grade} must be one character, not \"{key}\""));
            }
            if let Some((_, action)) = RESERVED.iter().find(|(reserved, _)| reserved == key) {
                return Err(format!("keys.{grade}: \"{key}\" is already used for {action}"));
            }
            if let Some((other, _)) = seen.iter().find(|(_, k)| k == key) {
                return Err(format!("keys.{grade}: \"{key}\" is already used for {other}"));
            }
            seen.push((grade, key.clone()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(text: &str) -> Result<[String; 4], String> {
        let k = parse(text)?.keys;
        Ok([k.again, k.hard, k.good, k.easy])
    }

    #[test]
    fn defaults_without_settings() {
        assert_eq!(keys("").unwrap(), ["1", "2", "3", "4"]);
        assert_eq!(keys("[keys]\n").unwrap(), ["1", "2", "3", "4"]);
    }

    #[test]
    fn reads_answer_keys() {
        let text = "[keys]\nagain = \"J\"\nhard = \"k\"\ngood = \"l\"\neasy = \";\"\n";
        assert_eq!(keys(text).unwrap(), ["j", "k", "l", ";"]);
        // Grades left out keep their default.
        assert_eq!(keys("[keys]\neasy = \"5\"\n").unwrap(), ["1", "2", "3", "5"]);
    }

    #[test]
    fn refuses_bad_keys() {
        assert_eq!(keys("[keys]\nagain = \"ab\"\n").unwrap_err(), "keys.again must be one character, not \"ab\"");
        assert_eq!(keys("[keys]\ngood = \" \"\n").unwrap_err(), "keys.good: \" \" is already used for showing the answer");
        assert_eq!(keys("[keys]\nhard = \"R\"\n").unwrap_err(), "keys.hard: \"r\" is already used for replaying sounds");
        assert_eq!(keys("[keys]\neasy = \"1\"\n").unwrap_err(), "keys.easy: \"1\" is already used for again");
        assert!(keys("[keys]\nagian = \"1\"\n").unwrap_err().contains("unknown field `agian`"));
    }
}
