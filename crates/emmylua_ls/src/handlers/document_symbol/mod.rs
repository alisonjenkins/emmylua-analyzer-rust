//! # document_symbol — pure-Semantic document symbols
//!
//! Syntax-tree traversal (functions / locals / assignments / table fields) plus type projection (`type_of_decl`).
//! The old DbIndex-based version (decl_tree hierarchy and type cache details) is retired; see migration notes §M4.

use emmylua_code_analysis::{DeclKind, LuaType, SemanticModel};
use emmylua_parser::{
    LuaAst, LuaAstNode, LuaAstToken, LuaSyntaxKind, LuaTableField, LuaVarExpr, PathTrait,
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
                let mut raw_name = func_name.get_access_path().unwrap_or_default();
                if let LuaVarExpr::IndexExpr(index_expr) = &func_name
                    && index_expr
                        .get_index_token()
                        .is_some_and(|token| token.is_colon())
                    && let Some(dot) = raw_name.rfind('.')
                {
                    raw_name.replace_range(dot..=dot, ":");
                }
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
                if local_name
                    .syntax()
                    .parent()
                    .is_some_and(|parent| parent.kind() == LuaSyntaxKind::LocalFuncStat.into())
                {
                    continue;
                }
                let Some(name_token) = local_name.get_name_token() else {
                    continue;
                };
                let Some(decl) = model.decl_by_offset(name_token.get_position()) else {
                    continue;
                };
                let ty = model.type_of_decl(&decl).unwrap_or(LuaType::Unknown);
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
            _ => continue,
        };
        if let Some(symbol) = symbol {
            symbols.push(symbol);
        }
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
    fn local_function_is_one_symbol() {
        let got = names(&symbols("local function foo(a) return a end\n"));
        assert_eq!(got, vec![("foo".to_string(), SymbolKind::FUNCTION)]);
    }

    #[test]
    fn local_with_unknown_type_is_listed() {
        let got = names(&symbols("local c = unknown_global_call()\n"));
        assert_eq!(got, vec![("c".to_string(), SymbolKind::VARIABLE)]);
    }

    #[test]
    fn colon_method_keeps_colon_in_name() {
        let got = names(&symbols(
            "local Reader = {}\nfunction Reader:close() end\nfunction Reader.open() end\n",
        ));
        assert!(
            got.contains(&("Reader:close".to_string(), SymbolKind::FUNCTION)),
            "{got:?}"
        );
        assert!(
            got.contains(&("Reader.open".to_string(), SymbolKind::FUNCTION)),
            "{got:?}"
        );
    }
}
