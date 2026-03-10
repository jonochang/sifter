use std::path::Path;

use sifter_codeintel::{CodeRelation, CodeSymbol, LanguagePlugin, RelationKind, SymbolKind};
use tree_sitter::Parser;

pub struct RustPlugin;

impl LanguagePlugin for RustPlugin {
    fn language_name(&self) -> &'static str {
        "rust"
    }

    fn matches_path(&self, path: &Path) -> bool {
        path.extension().and_then(|ext| ext.to_str()) == Some("rs")
    }

    fn extract_symbols(&self, source: &str, _path: &Path) -> Vec<CodeSymbol> {
        let tree = match parse_tree(source) {
            Some(tree) => tree,
            None => return Vec::new(),
        };

        let root = tree.root_node();
        let mut cursor = root.walk();
        root.children(&mut cursor)
            .filter_map(|node| extract_symbol(node, source))
            .collect()
    }

    fn extract_relations(&self, source: &str, _path: &Path) -> Vec<CodeRelation> {
        let tree = match parse_tree(source) {
            Some(tree) => tree,
            None => return Vec::new(),
        };

        let mut relations = Vec::new();
        collect_relations(tree.root_node(), source, &mut relations);
        relations
    }
}

fn parse_tree(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("load Rust grammar");
    parser.parse(source, None)
}

fn extract_symbol(node: tree_sitter::Node<'_>, source: &str) -> Option<CodeSymbol> {
    let kind = match node.kind() {
        "function_item" => SymbolKind::Function,
        "struct_item" => SymbolKind::Struct,
        "enum_item" => SymbolKind::Enum,
        "trait_item" => SymbolKind::Trait,
        "impl_item" => SymbolKind::Impl,
        "const_item" => SymbolKind::Constant,
        "type_item" => SymbolKind::TypeAlias,
        "mod_item" => SymbolKind::Module,
        _ => return None,
    };

    let name = if matches!(kind, SymbolKind::Impl) {
        impl_name(node, source)?
    } else {
        child_text(node, "name", source)?
    };

    Some(CodeSymbol {
        name,
        kind,
        line_start: node.start_position().row + 1,
        line_end: node.end_position().row + 1,
        scope: None,
    })
}

fn child_text(node: tree_sitter::Node<'_>, field: &str, source: &str) -> Option<String> {
    let child = node.child_by_field_name(field)?;
    Some(child.utf8_text(source.as_bytes()).ok()?.to_string())
}

fn impl_name(node: tree_sitter::Node<'_>, source: &str) -> Option<String> {
    if let Some(type_node) = node.child_by_field_name("type") {
        return Some(type_node.utf8_text(source.as_bytes()).ok()?.to_string());
    }

    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "type_identifier")
        .and_then(|child| child.utf8_text(source.as_bytes()).ok().map(str::to_string))
}

fn collect_relations(node: tree_sitter::Node<'_>, source: &str, relations: &mut Vec<CodeRelation>) {
    if node.kind() == "use_declaration" {
        for name in import_names(node, source) {
            relations.push(CodeRelation {
                name,
                kind: RelationKind::Import,
                line_start: node.start_position().row + 1,
                line_end: node.end_position().row + 1,
            });
        }
    } else if node.kind() == "type_identifier"
        && !is_definition_name(node)
        && let Ok(name) = node.utf8_text(source.as_bytes())
        && !name.is_empty()
    {
        relations.push(CodeRelation {
            name: name.to_string(),
            kind: RelationKind::Mention,
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_relations(child, source, relations);
    }
}

fn is_definition_name(node: tree_sitter::Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if !matches!(
        parent.kind(),
        "struct_item" | "enum_item" | "trait_item" | "type_item" | "mod_item"
    ) {
        return false;
    }

    parent
        .child_by_field_name("name")
        .is_some_and(|name| same_node(name, node))
}

fn same_node(left: tree_sitter::Node<'_>, right: tree_sitter::Node<'_>) -> bool {
    left.start_byte() == right.start_byte() && left.end_byte() == right.end_byte()
}

fn import_names(node: tree_sitter::Node<'_>, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    collect_import_names(node, source, &mut names);
    names.sort();
    names.dedup();
    names
}

fn collect_import_names(node: tree_sitter::Node<'_>, source: &str, names: &mut Vec<String>) {
    if matches!(node.kind(), "identifier" | "type_identifier")
        && let Ok(name) = node.utf8_text(source.as_bytes())
        && !matches!(name, "crate" | "self" | "super")
        && !name.is_empty()
    {
        names.push(name.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_import_names(child, source, names);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_top_level_symbols_with_line_ranges() {
        let source = r#"
pub struct RetryPolicy;
pub enum RetryMode { Fast }
pub trait Retryable {}
impl RetryPolicy {}
pub const DEFAULT_BUDGET: usize = 3;
pub type Budget = usize;
pub mod nested {}
pub fn retry_budget() -> usize { 3 }
"#;

        let plugin = RustPlugin;
        let symbols = plugin.extract_symbols(source, Path::new("src/retry.rs"));

        let summary = symbols
            .into_iter()
            .map(|symbol| {
                (
                    symbol.name,
                    symbol.kind.as_str().to_string(),
                    symbol.line_start,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            summary,
            vec![
                ("RetryPolicy".to_string(), "struct".to_string(), 2),
                ("RetryMode".to_string(), "enum".to_string(), 3),
                ("Retryable".to_string(), "trait".to_string(), 4),
                ("RetryPolicy".to_string(), "impl".to_string(), 5),
                ("DEFAULT_BUDGET".to_string(), "constant".to_string(), 6),
                ("Budget".to_string(), "type_alias".to_string(), 7),
                ("nested".to_string(), "module".to_string(), 8),
                ("retry_budget".to_string(), "function".to_string(), 9),
            ]
        );
    }

    #[test]
    fn extracts_imports_and_type_mentions_without_comment_noise() {
        let source = r#"
use crate::RetryPolicy;

// RetryPolicy is only a comment mention here.
pub fn build(policy: RetryPolicy) -> RetryPolicy { policy }
pub fn note() -> &'static str { "RetryPolicy" }
"#;

        let plugin = RustPlugin;
        let relations = plugin.extract_relations(source, Path::new("src/client.rs"));
        let summary = relations
            .into_iter()
            .map(|relation| {
                (
                    relation.name,
                    relation.kind.as_str().to_string(),
                    relation.line_start,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            summary,
            vec![
                ("RetryPolicy".to_string(), "import".to_string(), 2),
                ("RetryPolicy".to_string(), "mention".to_string(), 5),
                ("RetryPolicy".to_string(), "mention".to_string(), 5),
            ]
        );
    }
}
