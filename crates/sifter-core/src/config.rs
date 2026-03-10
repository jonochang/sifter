use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Config {
    #[serde(default)]
    pub global_context: Option<String>,
    #[serde(default)]
    pub collections: BTreeMap<String, Collection>,
    #[serde(default)]
    pub contexts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Collection {
    pub path: PathBuf,
    #[serde(default = "default_pattern")]
    pub pattern: String,
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub update: Option<String>,
    #[serde(default = "default_include_by_default", alias = "include_by_default")]
    #[serde(rename = "includeByDefault")]
    pub include_by_default: bool,
}

impl Default for Collection {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            pattern: default_pattern(),
            ignore: Vec::new(),
            context: None,
            update: None,
            include_by_default: default_include_by_default(),
        }
    }
}

fn default_pattern() -> String {
    "**/*".to_string()
}

const fn default_include_by_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMatch {
    pub scope: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(index_name: &str) -> Result<Self> {
        let path = config_file_path(index_name)?;
        Ok(Self { path })
    }

    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Config> {
        if !self.path.exists() {
            return Ok(Config::default());
        }

        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read config at {}", self.path.display()))?;
        let config = serde_yaml::from_str(&content)
            .with_context(|| format!("failed to parse config at {}", self.path.display()))?;
        Ok(config)
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let yaml = serde_yaml::to_string(config).context("failed to serialize config")?;
        fs::write(&self.path, yaml)
            .with_context(|| format!("failed to write config at {}", self.path.display()))?;
        Ok(())
    }

    pub fn add_collection(
        &self,
        name: &str,
        path: impl Into<PathBuf>,
        pattern: Option<String>,
    ) -> Result<Config> {
        let mut config = self.load()?;
        if config.collections.contains_key(name) {
            return Err(anyhow!("collection '{name}' already exists"));
        }

        let absolute_path = absolutize(path.into())?;
        config.collections.insert(
            name.to_string(),
            Collection {
                path: absolute_path,
                pattern: pattern.unwrap_or_else(default_pattern),
                ..Collection::default()
            },
        );
        self.save(&config)?;
        Ok(config)
    }

    pub fn remove_collection(&self, name: &str) -> Result<Config> {
        let mut config = self.load()?;
        if config.collections.remove(name).is_none() {
            return Err(anyhow!("collection '{name}' does not exist"));
        }
        self.save(&config)?;
        Ok(config)
    }

    pub fn rename_collection(&self, from: &str, to: &str) -> Result<Config> {
        let mut config = self.load()?;
        if config.collections.contains_key(to) {
            return Err(anyhow!("collection '{to}' already exists"));
        }
        let collection = config
            .collections
            .remove(from)
            .ok_or_else(|| anyhow!("collection '{from}' does not exist"))?;
        config.collections.insert(to.to_string(), collection);
        self.save(&config)?;
        Ok(config)
    }

    pub fn add_context(&self, scope: &str, value: &str) -> Result<Config> {
        let mut config = self.load()?;
        config.contexts.insert(scope.to_string(), value.to_string());
        self.save(&config)?;
        Ok(config)
    }

    pub fn remove_context(&self, scope: &str) -> Result<Config> {
        let mut config = self.load()?;
        config.contexts.remove(scope);
        self.save(&config)?;
        Ok(config)
    }
}

pub fn config_file_path(index_name: &str) -> Result<PathBuf> {
    if let Ok(path) = env::var("SIFTER_CONFIG_FILE") {
        return Ok(PathBuf::from(path));
    }

    if let Ok(dir) = env::var("SIFTER_CONFIG_HOME") {
        return Ok(PathBuf::from(dir).join(format!("{index_name}.yml")));
    }

    let project_dirs = ProjectDirs::from("", "", "sifter")
        .ok_or_else(|| anyhow!("failed to resolve XDG config directory"))?;
    Ok(project_dirs.config_dir().join(format!("{index_name}.yml")))
}

pub fn cache_file_path(index_name: &str) -> Result<PathBuf> {
    if let Ok(path) = env::var("SIFTER_CACHE_FILE") {
        return Ok(PathBuf::from(path));
    }

    if let Ok(dir) = env::var("SIFTER_CACHE_HOME") {
        return Ok(PathBuf::from(dir).join(format!("{index_name}.sqlite3")));
    }

    let project_dirs = ProjectDirs::from("", "", "sifter")
        .ok_or_else(|| anyhow!("failed to resolve XDG cache directory"))?;
    Ok(project_dirs
        .cache_dir()
        .join(format!("{index_name}.sqlite3")))
}

pub fn cache_dir_path(index_name: &str) -> Result<PathBuf> {
    if let Ok(path) = env::var("SIFTER_CACHE_DIR") {
        return Ok(PathBuf::from(path));
    }

    if let Ok(dir) = env::var("SIFTER_CACHE_HOME") {
        return Ok(PathBuf::from(dir).join(index_name));
    }

    let project_dirs = ProjectDirs::from("", "", "sifter")
        .ok_or_else(|| anyhow!("failed to resolve XDG cache directory"))?;
    Ok(project_dirs.cache_dir().join(index_name))
}

pub fn matching_contexts(config: &Config, candidate: &str) -> Vec<ContextMatch> {
    let mut matches = config
        .contexts
        .iter()
        .filter(|(scope, _)| {
            candidate == scope.as_str() || candidate.starts_with(&format!("{scope}/"))
        })
        .map(|(scope, value)| ContextMatch {
            scope: scope.clone(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| right.scope.len().cmp(&left.scope.len()));
    matches
}

fn absolutize(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }

    let cwd = env::current_dir().context("failed to read current directory")?;
    Ok(cwd.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_matching_prefers_longest_prefix() {
        let config = Config {
            contexts: BTreeMap::from([
                ("sifter://repo".to_string(), "root".to_string()),
                ("sifter://repo/src".to_string(), "source".to_string()),
            ]),
            ..Config::default()
        };

        let matches = matching_contexts(&config, "sifter://repo/src/main.rs");
        let scopes = matches
            .into_iter()
            .map(|item| item.scope)
            .collect::<Vec<_>>();
        assert_eq!(scopes, vec!["sifter://repo/src", "sifter://repo"]);
    }

    #[test]
    fn yaml_round_trip_uses_qmd_style_keys() {
        let config = Config {
            collections: BTreeMap::from([(
                "repo".to_string(),
                Collection {
                    path: PathBuf::from("/tmp/repo"),
                    include_by_default: true,
                    ..Collection::default()
                },
            )]),
            ..Config::default()
        };

        let yaml = serde_yaml::to_string(&config).expect("serialize config");
        assert!(yaml.contains("includeByDefault: true"));
    }

    #[test]
    fn cache_dir_uses_override_when_present() {
        let original = env::var_os("SIFTER_CACHE_DIR");
        unsafe {
            env::set_var("SIFTER_CACHE_DIR", "/tmp/sifter-cache-test");
        }

        let path = cache_dir_path("default").expect("cache path");
        assert_eq!(path, PathBuf::from("/tmp/sifter-cache-test"));

        match original {
            Some(value) => unsafe { env::set_var("SIFTER_CACHE_DIR", value) },
            None => unsafe { env::remove_var("SIFTER_CACHE_DIR") },
        }
    }
}
