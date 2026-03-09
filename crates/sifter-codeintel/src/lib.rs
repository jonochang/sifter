use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Constant,
    TypeAlias,
    Module,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Constant => "constant",
            Self::TypeAlias => "type_alias",
            Self::Module => "module",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub line_start: usize,
    pub line_end: usize,
    pub scope: Option<String>,
}

pub trait LanguagePlugin {
    fn language_name(&self) -> &'static str;
    fn matches_path(&self, path: &Path) -> bool;
    fn extract_symbols(&self, source: &str, path: &Path) -> Vec<CodeSymbol>;
}

#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn LanguagePlugin + Send + Sync>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<P>(&mut self, plugin: P)
    where
        P: LanguagePlugin + Send + Sync + 'static,
    {
        self.plugins.push(Box::new(plugin));
    }

    pub fn plugin_for_path(
        &self,
        path: &Path,
    ) -> Option<&(dyn LanguagePlugin + Send + Sync + 'static)> {
        self.plugins
            .iter()
            .find(|plugin| plugin.matches_path(path))
            .map(Box::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakePlugin;

    impl LanguagePlugin for FakePlugin {
        fn language_name(&self) -> &'static str {
            "fake"
        }

        fn matches_path(&self, path: &Path) -> bool {
            path.extension().and_then(|ext| ext.to_str()) == Some("fake")
        }

        fn extract_symbols(&self, _source: &str, _path: &Path) -> Vec<CodeSymbol> {
            vec![CodeSymbol {
                name: "Example".to_string(),
                kind: SymbolKind::Struct,
                line_start: 1,
                line_end: 1,
                scope: None,
            }]
        }
    }

    #[test]
    fn registry_selects_matching_plugin() {
        let mut registry = PluginRegistry::new();
        registry.register(FakePlugin);

        let plugin = registry
            .plugin_for_path(Path::new("demo.fake"))
            .expect("plugin");
        assert_eq!(plugin.language_name(), "fake");
    }
}
