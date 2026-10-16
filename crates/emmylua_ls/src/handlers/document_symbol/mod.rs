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
    DocumentSymbolResponse, OneOf, Range, ServerCapabilities, SymbolKind,
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
                    range: document
                        .to_lsp_range(func_stat.get_range())
                        .unwrap_or(lsp_range),
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
                    range: document
                        .to_lsp_range(local_func_stat.get_range())
                        .unwrap_or(lsp_range),
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
                    range: local_name
                        .syntax()
                        .parent()
                        .and_then(|stat| document.to_lsp_range(stat.text_range()))
                        .unwrap_or(lsp_range),
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
    nest_by_range(symbols)
}

/// Moves each symbol under the closest earlier symbol whose range strictly
/// contains it. `symbols` must be in document (pre-order) order.
fn nest_by_range(symbols: Vec<DocumentSymbol>) -> Vec<DocumentSymbol> {
    let contains = |outer: &Range, inner: &Range| {
        outer != inner
            && (outer.start.line, outer.start.character)
                <= (inner.start.line, inner.start.character)
            && (inner.end.line, inner.end.character) <= (outer.end.line, outer.end.character)
    };

    let mut parents: Vec<Option<usize>> = Vec::with_capacity(symbols.len());
    let mut open: Vec<usize> = Vec::new();
    for (index, symbol) in symbols.iter().enumerate() {
        while let Some(&top) = open.last() {
            if symbols
                .get(top)
                .is_some_and(|t| contains(&t.range, &symbol.range))
            {
                break;
            }
            open.pop();
        }
        parents.push(open.last().copied());
        open.push(index);
    }

    // Children always sit after their parent, so walking backwards finishes a
    // symbol's children before the symbol itself is attached to its parent.
    let mut slots: Vec<Option<DocumentSymbol>> = symbols.into_iter().map(Some).collect();
    let mut children: Vec<Vec<DocumentSymbol>> = slots.iter().map(|_| Vec::new()).collect();
    let mut roots = Vec::new();
    for index in (0..slots.len()).rev() {
        let Some(mut symbol) = slots.get_mut(index).and_then(Option::take) else {
            continue;
        };
        let mut own = children
            .get_mut(index)
            .map(std::mem::take)
            .unwrap_or_default();
        own.reverse();
        symbol.children = (!own.is_empty()).then_some(own);
        match parents.get(index).copied().flatten() {
            Some(parent) => {
                if let Some(siblings) = children.get_mut(parent) {
                    siblings.push(symbol);
                }
            }
            None => roots.push(symbol),
        }
    }
    roots.reverse();
    roots
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
        range: document
            .to_lsp_range(table_field.get_range())
            .unwrap_or(range),
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
    use lsp_types::{DocumentSymbol, Position, Range, SymbolKind};

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

    fn outline(symbols: &[DocumentSymbol], depth: usize, out: &mut Vec<String>) {
        for symbol in symbols {
            out.push(format!("{}{}", "  ".repeat(depth), symbol.name));
            outline(
                symbol.children.as_deref().unwrap_or_default(),
                depth + 1,
                out,
            );
        }
    }

    fn outline_of(source: &str) -> Vec<String> {
        let mut out = Vec::new();
        outline(&symbols(source), 0, &mut out);
        out
    }

    fn contains(outer: Range, inner: Range) -> bool {
        let before = |a: Position, b: Position| (a.line, a.character) <= (b.line, b.character);
        before(outer.start, inner.start) && before(inner.end, outer.end)
    }

    fn check_selection_inside_range(symbols: &[DocumentSymbol]) {
        for symbol in symbols {
            assert!(
                contains(symbol.range, symbol.selection_range),
                "{}: selection {:?} outside range {:?}",
                symbol.name,
                symbol.selection_range,
                symbol.range
            );
            check_selection_inside_range(symbol.children.as_deref().unwrap_or_default());
        }
    }

    #[test]
    fn locals_nest_under_their_function() {
        let got = outline_of(
            "function outer()\n  local a = 1\n  function inner()\n    local b = 2\n  end\nend\nlocal top = 3\n",
        );
        assert_eq!(got, ["outer", "  a", "  inner", "    b", "top"]);
    }

    #[test]
    fn table_fields_nest_under_their_table() {
        let got = outline_of("local t = {\n  a = 1,\n  b = function() local x = 1 end,\n}\n");
        assert_eq!(got, ["t", "  a", "  b", "    x"]);
    }

    #[test]
    fn selection_range_is_inside_range() {
        let symbols = symbols(
            "local M = {}\nfunction M.f(a)\n  local x = a\nend\nlocal t = { k = 1 }\nlocal p, q = 1, 2\n",
        );
        check_selection_inside_range(&symbols);
    }
}
