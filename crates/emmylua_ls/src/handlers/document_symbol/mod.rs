//! # document_symbol — pure-Semantic document symbols
//!
//! Syntax-tree traversal (functions / locals / assignments / table fields) plus type projection (`type_of_decl`).
//! The old DbIndex-based version (decl_tree hierarchy and type cache details) is retired; see migration notes §M4.

use emmylua_code_analysis::{DeclKind, LuaType, SemanticModel};
use emmylua_parser::{
    LuaAssignStat, LuaAst, LuaAstNode, LuaAstToken, LuaExpr, LuaIndexKey, LuaTableField,
    LuaVarExpr, PathTrait,
};
use lsp_types::{
    ClientCapabilities, DocumentSymbol, DocumentSymbolOptions, DocumentSymbolParams,
    DocumentSymbolResponse, OneOf, ServerCapabilities, SymbolKind,
};
use tokio_util::sync::CancellationToken;

use crate::context::{RequestOutcome, ServerContextSnapshot, analysis_query};
use crate::handlers::hover::render::humanize;

use super::RegisterCapabilities;

pub async fn on_document_symbol(
    context: ServerContextSnapshot,
    params: DocumentSymbolParams,
    cancel_token: CancellationToken,
) -> RequestOutcome<DocumentSymbolResponse> {
    let uri = params.text_document.uri;
    let cache_key = format!("doc_symbol:{}", uri.as_str());
    analysis_query(
        context.analysis(),
        context.request_manager(),
        &cache_key,
        Some(cancel_token.clone()),
        move |analysis| {
            let file_id = analysis.get_file_id(&uri)?;
            let model = analysis.semantic_model(file_id);
            let document = analysis.db.document(file_id)?;
            Some(DocumentSymbolResponse::Nested(build_document_symbol(
                &model, &document,
            )))
        },
    )
    .await
}

fn non_empty_symbol_name(raw: String, fallback: impl FnOnce() -> String) -> String {
    let name = raw.trim();
    if name.is_empty() {
        let fallback = fallback();
        let fallback = fallback.trim();
        if fallback.is_empty() {
            "(anonymous)".to_string()
        } else {
            fallback.to_string()
        }
    } else {
        name.to_string()
    }
}

fn build_document_symbol(
    model: &SemanticModel<'_>,
    document: &emmylua_code_analysis::LuaDocument,
) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();
    let Some(chunk) = model.chunk() else {
        return symbols;
    };
    for node in chunk.descendants::<LuaAst>() {
        let symbol = match &node {
            LuaAst::LuaFuncStat(func_stat) => {
                let Some(func_name) = func_stat.get_func_name() else {
                    continue;
                };
                let Some(lsp_range) = document.to_lsp_range(func_name.get_range()) else {
                    continue;
                };
                let raw_name = func_name.get_access_path().unwrap_or_default();
                let name = non_empty_symbol_name(raw_name, || {
                    func_name.syntax().text().to_string().trim().to_string()
                });
                Some(DocumentSymbol {
                    name,
                    detail: None,
                    kind: SymbolKind::FUNCTION,
                    range: lsp_range,
                    selection_range: lsp_range,
                    children: None,
                    tags: None,
                    #[allow(deprecated)]
                    deprecated: None,
                })
            }
            LuaAst::LuaLocalFuncStat(local_func_stat) => {
                let Some(func_name) = local_func_stat.get_local_name() else {
                    continue;
                };
                let Some(lsp_range) = document.to_lsp_range(func_name.get_range()) else {
                    continue;
                };
                let name = non_empty_symbol_name(
                    func_name
                        .get_name_token()
                        .map(|t| t.get_name_text().to_string())
                        .unwrap_or_default(),
                    || func_name.syntax().text().to_string().trim().to_string(),
                );
                Some(DocumentSymbol {
                    name,
                    detail: None,
                    kind: SymbolKind::FUNCTION,
                    range: lsp_range,
                    selection_range: lsp_range,
                    children: None,
                    tags: None,
                    #[allow(deprecated)]
                    deprecated: None,
                })
            }
            LuaAst::LuaLocalName(local_name) => {
                let Some(name_token) = local_name.get_name_token() else {
                    continue;
                };
                let Some(decl) = model.decl_by_offset(name_token.get_position()) else {
                    continue;
                };
                let ty = model.type_of_decl(&decl).unwrap_or(LuaType::Unknown);
                if matches!(ty, LuaType::Unknown) {
                    continue;
                }
                let Some(lsp_range) = document.to_lsp_range(name_token.get_range()) else {
                    continue;
                };
                Some(DocumentSymbol {
                    name: name_token.get_name_text().to_string(),
                    detail: (!matches!(ty, LuaType::Unknown)).then(|| humanize(model, &ty)),
                    kind: match decl_kind(model, &decl) {
                        Some(DeclKind::Local { .. }) | Some(DeclKind::Param) => {
                            SymbolKind::VARIABLE
                        }
                        _ => SymbolKind::VARIABLE,
                    },
                    range: lsp_range,
                    selection_range: lsp_range,
                    children: None,
                    tags: None,
                    #[allow(deprecated)]
                    deprecated: None,
                })
            }
            LuaAst::LuaTableField(table_field) => {
                build_table_field_symbol(model, document, table_field)
            }
            LuaAst::LuaAssignStat(assign_stat) => {
                symbols.extend(build_assign_symbols(model, document, assign_stat));
                continue;
            }
            _ => continue,
        };
        if let Some(symbol) = symbol {
            symbols.push(symbol);
        }
    }
    symbols
}

/// One symbol per `a.b = v` / global `G = v` target. A plain name that is not
/// a declaration is a reassignment of a local and adds nothing to the outline.
fn build_assign_symbols(
    model: &SemanticModel<'_>,
    document: &emmylua_code_analysis::LuaDocument,
    assign_stat: &LuaAssignStat,
) -> Vec<DocumentSymbol> {
    let Some(stat_range) = document.to_lsp_range(assign_stat.get_range()) else {
        return Vec::new();
    };
    let (vars, values) = assign_stat.get_var_and_expr_list();
    let mut symbols = Vec::new();
    for (index, var) in vars.iter().enumerate() {
        let name = match var {
            LuaVarExpr::NameExpr(name_expr) => {
                if model.decl_by_offset(name_expr.get_position()).is_none() {
                    continue;
                }
                name_expr.get_access_path()
            }
            LuaVarExpr::IndexExpr(index_expr) => {
                if !matches!(
                    index_expr.get_index_key(),
                    Some(LuaIndexKey::Name(_) | LuaIndexKey::String(_))
                ) {
                    continue;
                }
                index_expr.get_access_path()
            }
        };
        let Some(name) = name else {
            continue;
        };
        let Some(selection_range) = document.to_lsp_range(var.get_range()) else {
            continue;
        };
        let value = values.get(index);
        let ty = value
            .map(|expr| model.type_of_expr(expr.get_syntax_id()))
            .unwrap_or(LuaType::Unknown);
        let is_function = matches!(value, Some(LuaExpr::ClosureExpr(_)))
            || matches!(ty, LuaType::DocFunction(_) | LuaType::Function);
        symbols.push(DocumentSymbol {
            name,
            detail: (!matches!(ty, LuaType::Unknown)).then(|| humanize(model, &ty)),
            kind: if is_function {
                SymbolKind::FUNCTION
            } else {
                SymbolKind::FIELD
            },
            range: stat_range,
            selection_range,
            children: None,
            tags: None,
            #[allow(deprecated)]
            deprecated: None,
        });
    }
    symbols
}

fn decl_kind(
    model: &SemanticModel<'_>,
    decl: &emmylua_code_analysis::SemanticId,
) -> Option<DeclKind> {
    model
        .decls()
        .and_then(|decls| decls.iter().find(|d| &d.id == decl))
        .map(|d| d.kind)
}

fn build_table_field_symbol(
    model: &SemanticModel<'_>,
    document: &emmylua_code_analysis::LuaDocument,
    table_field: &LuaTableField,
) -> Option<DocumentSymbol> {
    if !table_field.is_assign_field() {
        return None;
    }
    let key = table_field.get_field_key()?;
    let name = key.get_path_part();
    let range = document.to_lsp_range(key.get_range()?)?;
    let ty = table_field
        .get_value_expr()
        .map(|expr| model.type_of_expr(expr.get_syntax_id()))
        .unwrap_or(LuaType::Unknown);
    Some(DocumentSymbol {
        name,
        detail: (!matches!(ty, LuaType::Unknown)).then(|| humanize(model, &ty)),
        kind: if matches!(ty, LuaType::DocFunction(_) | LuaType::Function) {
            SymbolKind::FUNCTION
        } else {
            SymbolKind::FIELD
        },
        range,
        selection_range: range,
        children: None,
        tags: None,
        #[allow(deprecated)]
        deprecated: None,
    })
}

pub struct DocumentSymbolCapabilities;

impl RegisterCapabilities for DocumentSymbolCapabilities {
    fn register_capabilities(server_capabilities: &mut ServerCapabilities, _: &ClientCapabilities) {
        server_capabilities.document_symbol_provider = Some(OneOf::Right(DocumentSymbolOptions {
            label: None,
            work_done_progress_options: Default::default(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::build_document_symbol;
    use crate::handlers::test_lib::ProviderVirtualWorkspace;
    use lsp_types::{DocumentSymbol, SymbolKind};

    fn symbols(source: &str) -> Vec<DocumentSymbol> {
        let mut ws = ProviderVirtualWorkspace::new();
        let file_id = ws.def(source);
        let model = ws.analysis.semantic_model(file_id);
        let document = ws.analysis.db.document(file_id).expect("document");
        build_document_symbol(&model, &document)
    }

    fn names(symbols: &[DocumentSymbol]) -> Vec<(String, SymbolKind)> {
        symbols.iter().map(|s| (s.name.clone(), s.kind)).collect()
    }

    #[test]
    fn field_assignments_are_listed() {
        let got = names(&symbols(
            "local M = {}\nM.f = function() end\nM.x = 1\nfunction M.g() end\n",
        ));
        assert_eq!(
            got,
            [
                ("M".to_string(), SymbolKind::VARIABLE),
                ("M.f".to_string(), SymbolKind::FUNCTION),
                ("M.x".to_string(), SymbolKind::FIELD),
                ("M.g".to_string(), SymbolKind::FUNCTION),
            ]
        );
    }

    #[test]
    fn global_assignment_is_listed() {
        let got = names(&symbols("G = function() end\n"));
        assert_eq!(got, [("G".to_string(), SymbolKind::FUNCTION)]);
    }

    #[test]
    fn local_reassignment_is_not_a_new_symbol() {
        let got = names(&symbols("local i = 1\ni = i + 1\n"));
        assert_eq!(got, [("i".to_string(), SymbolKind::VARIABLE)]);
    }

    #[test]
    fn dynamic_index_assignment_is_skipped() {
        let got = names(&symbols("local t = {}\nlocal k = 'a'\nt[k] = 1\n"));
        assert_eq!(
            got,
            [
                ("t".to_string(), SymbolKind::VARIABLE),
                ("k".to_string(), SymbolKind::VARIABLE),
            ]
        );
    }
}
