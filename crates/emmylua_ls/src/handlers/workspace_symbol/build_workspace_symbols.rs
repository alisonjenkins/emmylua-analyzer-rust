use std::{collections::HashSet, sync::Mutex};

use emmylua_code_analysis::{
    Decl, DeclKind, EmmyLuaAnalysis, FileFacts, LuaDocument, Member, SemanticId, SemanticModel,
};
use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaSyntaxKind, LuaSyntaxNode};
use lsp_types::{SymbolKind, SymbolTag, Uri, WorkspaceSymbol, WorkspaceSymbolResponse};
use tokio_util::sync::CancellationToken;

/// LuaLS stops at 1000 results; a one-letter query otherwise returns tens of thousands of symbols.
const MAX_WORKSPACE_SYMBOLS: usize = 1000;

/// if query contains uppercase, do case-sensitive match; otherwise, ignore case
fn match_symbol(text: &str, query: &str) -> bool {
    if query.chars().any(|c| c.is_uppercase()) {
        text.contains(query)
    } else if query.is_ascii() && text.is_ascii() {
        // Runs once per name use in the workspace; skip allocating a lowercased copy.
        query.is_empty()
            || text
                .as_bytes()
                .windows(query.len())
                .any(|window| window.eq_ignore_ascii_case(query.as_bytes()))
    } else {
        text.to_lowercase().contains(&query.to_lowercase())
    }
}

pub fn build_workspace_symbols(
    analysis: &EmmyLuaAnalysis,
    query: String,
    cancel_token: CancellationToken,
) -> Option<WorkspaceSymbolResponse> {
    let symbols = Mutex::new(Vec::new());
    analysis.db.parallel_for_each_file(|file_id, model| {
        if cancel_token.is_cancelled() {
            return;
        }
        let Some(document) = model.document(file_id) else {
            return;
        };
        let Some(uri) = document.get_uri() else {
            return;
        };
        let mut symbols = symbols.lock().expect("workspace symbols lock");
        if let Some(decls) = model.decls() {
            for decl in decls.iter() {
                if cancel_token.is_cancelled() || symbols.len() >= MAX_WORKSPACE_SYMBOLS {
                    return;
                }
                let kind = match decl.kind {
                    DeclKind::Global => SymbolKind::FUNCTION,
                    DeclKind::Local { .. } | DeclKind::Param => {
                        if decl.is_implicit_self || decl.name == "..." {
                            continue;
                        }
                        local_symbol_kind(decl)
                    }
                };
                if !query.is_empty() && !match_symbol(&decl.name, &query) {
                    continue;
                }
                let Some(lsp_range) = document.to_lsp_range(decl.name_range) else {
                    continue;
                };
                symbols.push(WorkspaceSymbol {
                    name: decl.name.to_string(),
                    kind,
                    tags: None,
                    location: lsp_types::OneOf::Left(lsp_types::Location {
                        uri: uri.clone(),
                        range: lsp_range,
                    }),
                    container_name: None,
                    data: None,
                });
            }
        }
        push_reassignments(model, &document, &uri, &query, &cancel_token, &mut symbols);
        if let Some(facts) = model.file_facts() {
            for def in facts.type_defs.iter() {
                if symbols.len() >= MAX_WORKSPACE_SYMBOLS {
                    return;
                }
                if !query.is_empty() && !match_symbol(&def.full_name, &query) {
                    continue;
                }
                let Some(lsp_range) = document.to_lsp_range(def.name_range) else {
                    continue;
                };
                symbols.push(WorkspaceSymbol {
                    name: def.full_name.to_string(),
                    kind: SymbolKind::CLASS,
                    tags: None,
                    location: lsp_types::OneOf::Left(lsp_types::Location {
                        uri: uri.clone(),
                        range: lsp_range,
                    }),
                    container_name: None,
                    data: None,
                });
            }
            // `local t = { a = 1 }` is collected twice (under the local and under the
            // anonymous table) with the same id; keep the first, which names the local.
            let mut seen_members = HashSet::new();
            for member in facts.members.iter() {
                if cancel_token.is_cancelled() || symbols.len() >= MAX_WORKSPACE_SYMBOLS {
                    return;
                }
                if !seen_members.insert(&member.id) {
                    continue;
                }
                let Some(name) = member.key.get_name() else {
                    continue;
                };
                if !query.is_empty() && !match_symbol(name, &query) {
                    continue;
                }
                let Some(key_range) = member.id.member_key_range() else {
                    continue;
                };
                let Some(lsp_range) = document.to_lsp_range(key_range) else {
                    continue;
                };
                symbols.push(WorkspaceSymbol {
                    name: name.to_string(),
                    kind: member_symbol_kind(member),
                    tags: member.deprecated.then(|| vec![SymbolTag::DEPRECATED]),
                    location: lsp_types::OneOf::Left(lsp_types::Location {
                        uri: uri.clone(),
                        range: lsp_range,
                    }),
                    container_name: member_container_name(facts, member),
                    data: None,
                });
            }
        }
    });
    Some(WorkspaceSymbolResponse::Nested(
        symbols.into_inner().expect("workspace symbols lock"),
    ))
}

/// LuaLS lists every plain assignment to a name (`plan = patch()`) next to its
/// declaration, whatever the name resolves to. Only name uses whose text matches
/// the query have their syntax node looked up.
fn push_reassignments(
    model: &SemanticModel<'_>,
    document: &LuaDocument<'_>,
    uri: &Uri,
    query: &str,
    cancel_token: &CancellationToken,
    symbols: &mut Vec<WorkspaceSymbol>,
) {
    let (Some(uses), Some(tree)) = (model.name_uses(), model.syntax_tree()) else {
        return;
    };
    let mut root = None;
    let mut decl_starts = None;
    for name_use in uses {
        if cancel_token.is_cancelled() || symbols.len() >= MAX_WORKSPACE_SYMBOLS {
            return;
        }
        if !query.is_empty() && !match_symbol(&name_use.name, query) {
            continue;
        }
        let root = root.get_or_insert_with(|| tree.get_red_root());
        let Some(node) = name_use.syntax.to_node_from_root(root) else {
            continue;
        };
        if !is_assignment_target(&node) {
            continue;
        }
        // The first assignment to a global is already its declaration.
        let decl_starts = decl_starts.get_or_insert_with(|| {
            model
                .decls()
                .unwrap_or_default()
                .iter()
                .map(|decl| decl.name_range.start())
                .collect::<HashSet<_>>()
        });
        let range = name_use.syntax.get_range();
        if decl_starts.contains(&range.start()) {
            continue;
        }
        let Some(lsp_range) = document.to_lsp_range(range) else {
            continue;
        };
        symbols.push(WorkspaceSymbol {
            name: name_use.name.to_string(),
            kind: SymbolKind::VARIABLE,
            tags: None,
            location: lsp_types::OneOf::Left(lsp_types::Location {
                uri: uri.clone(),
                range: lsp_range,
            }),
            container_name: None,
            data: None,
        });
    }
}

fn is_assignment_target(node: &LuaSyntaxNode) -> bool {
    node.parent()
        .and_then(LuaAssignStat::cast)
        .is_some_and(|assign| {
            let (vars, _) = assign.get_var_and_expr_list();
            vars.iter().any(|var| var.syntax() == node)
        })
}

fn member_symbol_kind(member: &Member) -> SymbolKind {
    if member.is_method {
        return SymbolKind::METHOD;
    }
    let is_function = member.value_syntax.is_some_and(|syntax| {
        matches!(
            syntax.get_kind(),
            LuaSyntaxKind::ClosureExpr | LuaSyntaxKind::TypeFun
        )
    });
    if is_function {
        SymbolKind::FUNCTION
    } else {
        SymbolKind::FIELD
    }
}

fn member_container_name(facts: &FileFacts, member: &Member) -> Option<String> {
    match &member.owner {
        SemanticId::TypeDef(key) => Some(key.full_name.to_string()),
        SemanticId::Name(name) => Some(name.to_string()),
        SemanticId::Decl(_) => facts
            .decl_by_id(&member.owner)
            .map(|decl| decl.name.to_string()),
        SemanticId::Member(_) => facts
            .member_by_id(&member.owner)
            .and_then(|owner| owner.key.get_name())
            .map(str::to_string),
        SemanticId::Signature(_) => None,
    }
}

// LuaLS reports every local as Variable; functions are told apart here so
// clients can filter on them.
fn local_symbol_kind(decl: &Decl) -> SymbolKind {
    let is_function = decl
        .value_expr_syntax
        .is_some_and(|syntax| syntax.get_kind() == LuaSyntaxKind::ClosureExpr);
    if is_function {
        SymbolKind::FUNCTION
    } else {
        SymbolKind::VARIABLE
    }
}
