//! # document_symbol — pure-Semantic document symbols
//!
//! Syntax-tree traversal (functions / locals / assignments / table fields) plus type projection (`type_of_decl`).
//! The old DbIndex-based version (decl_tree hierarchy and type cache details) is retired; see migration notes §M4.

use std::collections::HashMap;

use emmylua_code_analysis::{LuaType, SemanticModel};
use emmylua_parser::{
    LuaAssignStat, LuaAst, LuaAstNode, LuaAstToken, LuaCallExpr, LuaExpr, LuaIndexExpr,
    LuaIndexKey, LuaLiteralToken, LuaLocalStat, LuaSyntaxKind, LuaTableExpr, LuaTableField,
    LuaVarExpr, PathTrait,
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

/// LuaLS picks the symbol kind from the syntax of the assigned value, not its
/// inferred type, so `local f = other` stays a variable.
fn value_kind(value: Option<&LuaExpr>) -> SymbolKind {
    match value {
        Some(LuaExpr::LiteralExpr(literal)) => match literal.get_literal() {
            Some(LuaLiteralToken::String(_)) => SymbolKind::STRING,
            Some(LuaLiteralToken::Number(_)) => SymbolKind::NUMBER,
            Some(LuaLiteralToken::Bool(_)) => SymbolKind::BOOLEAN,
            _ => SymbolKind::VARIABLE,
        },
        Some(LuaExpr::TableExpr(table)) => {
            let mut fields = table.get_fields().peekable();
            if fields.peek().is_some() && fields.all(|field| field.is_value_field()) {
                SymbolKind::ARRAY
            } else {
                SymbolKind::OBJECT
            }
        }
        Some(LuaExpr::ClosureExpr(_)) => SymbolKind::FUNCTION,
        _ => SymbolKind::VARIABLE,
    }
}

/// Only `a.b = v` names a field; LuaLS treats `a["b"] = v` like any other
/// index assignment, which names nothing.
fn is_field_target(index_expr: &LuaIndexExpr) -> bool {
    matches!(index_expr.get_index_key(), Some(LuaIndexKey::Name(_)))
}

/// LuaLS sends blank text for a table or closure that nothing names.
const BLANK_NAME: &str = " ";

/// The name of a closure or table that is not the value of a named symbol:
/// `return` for returned values, the callee for call arguments, blank
/// otherwise. `None` when the value already belongs to a named symbol.
fn anonymous_head(expr: &LuaExpr) -> Option<String> {
    match LuaAst::cast(expr.syntax().parent()?)? {
        LuaAst::LuaFuncStat(_) | LuaAst::LuaLocalFuncStat(_) | LuaAst::LuaTableField(_) => None,
        LuaAst::LuaLocalStat(stat) => {
            let index = stat
                .get_value_exprs()
                .position(|value| value.syntax() == expr.syntax())?;
            (index >= stat.get_local_name_list().count()).then(String::new)
        }
        LuaAst::LuaAssignStat(stat) => {
            let (vars, values) = stat.get_var_and_expr_list();
            let index = values
                .iter()
                .position(|value| value.syntax() == expr.syntax())?;
            let named = match vars.get(index) {
                Some(LuaVarExpr::NameExpr(_)) => true,
                Some(LuaVarExpr::IndexExpr(index_expr)) => is_field_target(index_expr),
                None => false,
            };
            (!named).then(String::new)
        }
        LuaAst::LuaCallArgList(args) => {
            let callee = args.get_parent::<LuaCallExpr>()?.get_prefix_expr()?;
            let text = callee.syntax().text().to_string();
            Some(match text.trim().rsplit_once('\n') {
                Some((_, last)) => format!("... {}", last.trim()),
                None => text.trim().to_string(),
            })
        }
        LuaAst::LuaReturnStat(_) => Some("return".to_string()),
        _ => Some(String::new()),
    }
}

fn build_anonymous_symbol(
    document: &emmylua_code_analysis::LuaDocument,
    expr: LuaExpr,
) -> Option<DocumentSymbol> {
    let head = anonymous_head(&expr)?;
    let range = document.to_lsp_range(expr.get_range())?;
    let selection_range = match &expr {
        LuaExpr::ClosureExpr(_) => expr
            .syntax()
            .first_token()
            .and_then(|keyword| document.to_lsp_range(keyword.text_range()))
            .unwrap_or(range),
        _ => range,
    };
    Some(DocumentSymbol {
        name: if head.is_empty() {
            BLANK_NAME.to_string()
        } else {
            head
        },
        detail: None,
        kind: value_kind(Some(&expr)),
        range,
        selection_range,
        children: None,
        tags: None,
        #[allow(deprecated)]
        deprecated: None,
    })
}

fn local_name_value(local_name: &emmylua_parser::LuaLocalName) -> Option<LuaExpr> {
    let stat = local_name.get_parent::<LuaLocalStat>()?;
    let index = stat
        .get_local_name_list()
        .position(|name| name.syntax() == local_name.syntax())?;
    stat.get_value_exprs().nth(index)
}

fn build_document_symbol(
    model: &SemanticModel<'_>,
    document: &emmylua_code_analysis::LuaDocument,
) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();
    let Some(chunk) = model.chunk() else {
        return symbols;
    };
    let mut positional = PositionalIndex::new();
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
                let mut kind = SymbolKind::FUNCTION;
                if let LuaVarExpr::IndexExpr(index_expr) = &func_name
                    && index_expr
                        .get_index_token()
                        .is_some_and(|token| token.is_colon())
                    && let Some(dot) = raw_name.rfind('.')
                {
                    raw_name.replace_range(dot..=dot, ":");
                    kind = SymbolKind::METHOD;
                }
                let name = non_empty_symbol_name(raw_name, || {
                    func_name.syntax().text().to_string().trim().to_string()
                });
                Some(DocumentSymbol {
                    name,
                    detail: None,
                    kind,
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
                    kind: value_kind(local_name_value(local_name).as_ref()),
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
                build_table_field_symbol(model, document, table_field, &positional)
            }
            LuaAst::LuaAssignStat(assign_stat) => {
                symbols.extend(build_assign_symbols(model, document, assign_stat));
                continue;
            }
            LuaAst::LuaClosureExpr(closure) => {
                build_anonymous_symbol(document, LuaExpr::ClosureExpr(closure.clone()))
            }
            LuaAst::LuaTableExpr(table) => {
                index_positional_fields(table, &mut positional);
                build_anonymous_symbol(document, LuaExpr::TableExpr(table.clone()))
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

/// One symbol per `a.b = v` / `name = v` target, reassignments of locals and
/// undeclared globals included.
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
            LuaVarExpr::NameExpr(name_expr) => name_expr.get_access_path(),
            LuaVarExpr::IndexExpr(index_expr) => {
                if !is_field_target(index_expr) {
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
        symbols.push(DocumentSymbol {
            name,
            detail: (!matches!(ty, LuaType::Unknown)).then(|| humanize(model, &ty)),
            kind: value_kind(value),
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

/// Maps the start offset of each positional field to its 1-based index, so a
/// field's index is not recomputed by scanning its siblings.
type PositionalIndex = HashMap<u32, usize>;

fn index_positional_fields(table: &LuaTableExpr, positional: &mut PositionalIndex) {
    let fields = table.get_fields().filter(LuaTableField::is_value_field);
    for (index, field) in (1..).zip(fields) {
        positional.insert(u32::from(field.get_position()), index);
    }
}

fn build_table_field_symbol(
    model: &SemanticModel<'_>,
    document: &emmylua_code_analysis::LuaDocument,
    table_field: &LuaTableField,
    positional: &PositionalIndex,
) -> Option<DocumentSymbol> {
    if table_field.is_value_field() {
        let index = positional.get(&u32::from(table_field.get_position()))?;
        let value = table_field.get_value_expr()?;
        return Some(DocumentSymbol {
            name: format!("[{index}]"),
            detail: None,
            kind: value_kind(Some(&value)),
            range: document.to_lsp_range(table_field.get_range())?,
            selection_range: document.to_lsp_range(value.get_range())?,
            children: None,
            tags: None,
            #[allow(deprecated)]
            deprecated: None,
        });
    }
    let key = table_field.get_field_key()?;
    let name = match &key {
        LuaIndexKey::String(token) => format!("[{}]", token.syntax().text()),
        _ => key.get_path_part(),
    };
    let range = document.to_lsp_range(key.get_range()?)?;
    let ty = table_field
        .get_value_expr()
        .map(|expr| model.type_of_expr(expr.get_syntax_id()))
        .unwrap_or(LuaType::Unknown);
    Some(DocumentSymbol {
        name,
        detail: (!matches!(ty, LuaType::Unknown)).then(|| humanize(model, &ty)),
        kind: value_kind(table_field.get_value_expr().as_ref()),
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
            got.contains(&("Reader:close".to_string(), SymbolKind::METHOD)),
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

    #[test]
    fn field_assignments_are_listed() {
        let got = names(&symbols(
            "local M = {}\nM.f = function() end\nM.x = 1\nfunction M.g() end\n",
        ));
        assert_eq!(
            got,
            [
                ("M".to_string(), SymbolKind::OBJECT),
                ("M.f".to_string(), SymbolKind::FUNCTION),
                ("M.x".to_string(), SymbolKind::NUMBER),
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
    fn local_reassignment_is_listed_like_luals() {
        let got = names(&symbols("local i = 1\ni = i + 1\nj = {}\n"));
        assert_eq!(
            got,
            [
                ("i".to_string(), SymbolKind::NUMBER),
                ("i".to_string(), SymbolKind::VARIABLE),
                ("j".to_string(), SymbolKind::OBJECT),
            ]
        );
    }

    #[test]
    fn dynamic_index_assignment_is_skipped() {
        let got = names(&symbols("local t = {}\nlocal k = 'a'\nt[k] = 1\n"));
        assert_eq!(
            got,
            [
                ("t".to_string(), SymbolKind::OBJECT),
                ("k".to_string(), SymbolKind::STRING),
            ]
        );
    }

    #[test]
    fn call_argument_closure_is_named_after_the_callee() {
        let got = outline_of(
            "vim.schedule(function()\n  local x = 1\nend)\nclient:request('m', function(err) end)\ndescribe('a', function() end)\n",
        );
        assert_eq!(got, ["vim.schedule", "  x", "client:request", "describe"]);
        let kinds = names(&symbols("foo(function() end)\n"));
        assert_eq!(kinds, [("foo".to_string(), SymbolKind::FUNCTION)]);
    }

    #[test]
    fn call_argument_table_is_named_after_the_callee() {
        let got = outline_of("setmetatable(M, { __index = Base })\nopts({ a = 1 })\nf{ b = 2 }\n");
        assert_eq!(
            got,
            ["setmetatable", "  __index", "opts", "  a", "f", "  b"]
        );
        let kinds = names(&symbols("foo({ 1, 2 })\nbar({ k = 1 })\n"));
        assert_eq!(
            kinds,
            [
                ("foo".to_string(), SymbolKind::ARRAY),
                ("bar".to_string(), SymbolKind::OBJECT)
            ]
        );
    }

    #[test]
    fn returned_closure_and_table_are_named_return() {
        let got =
            outline_of("local function f()\n  return function() end\nend\nreturn { a = 1 }\n");
        assert_eq!(got, ["f", "  return", "return", "  a"]);
    }

    #[test]
    fn unnamed_table_operand_gets_a_blank_symbol() {
        let operand = symbols("local x = y or {}\n");
        let blank = operand
            .first()
            .and_then(|x| x.children.as_deref())
            .and_then(|children| children.first())
            .map(|symbol| (symbol.name.as_str(), symbol.kind));
        assert_eq!(blank, Some((" ", SymbolKind::OBJECT)));

        let dynamic = names(&symbols("t[k] = {}\n"));
        assert_eq!(dynamic, [(" ".to_string(), SymbolKind::OBJECT)]);
    }

    #[test]
    fn values_that_name_a_symbol_are_not_listed_twice() {
        let got = outline_of(
            "local a = {}\nlocal f = function() end\nM.g = function() end\nfunction M.h() end\nlocal t = { k = function() end, o = {} }\n",
        );
        assert_eq!(got, ["a", "f", "M.g", "M.h", "t", "  k", "  o"]);
    }

    #[test]
    fn anonymous_selection_range_is_inside_range() {
        let symbols = symbols("foo(function()\n  local x = 1\nend, { a = 1 })\nreturn {}\n");
        check_selection_inside_range(&symbols);
    }

    #[test]
    fn positional_items_are_listed_by_index() {
        let got = outline_of("local t = { 'a', 2, f = 1, g() }\n");
        assert_eq!(got, ["t", "  [1]", "  [2]", "  f", "  [3]"]);
        let kinds = symbols("local t = { 'a', 2, true, {}, { 1 }, function() end, g() }\n");
        let kinds: Vec<_> = kinds[0]
            .children
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|symbol| symbol.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                SymbolKind::STRING,
                SymbolKind::NUMBER,
                SymbolKind::BOOLEAN,
                SymbolKind::OBJECT,
                SymbolKind::ARRAY,
                SymbolKind::FUNCTION,
                SymbolKind::VARIABLE,
            ]
        );
    }

    #[test]
    fn positional_items_nest_their_contents() {
        let got = outline_of("local t = {\n  { a = 1 },\n  function() local x = 1 end,\n}\n");
        assert_eq!(got, ["t", "  [1]", "    a", "  [2]", "    x"]);
    }

    #[test]
    fn long_positional_list_keeps_counting() {
        let source = format!("local t = {{ {} }}\n", "'x', ".repeat(3000));
        let all = symbols(&source);
        let items = all[0].children.as_deref().unwrap_or_default();
        assert_eq!(items.len(), 3000);
        assert_eq!(items.last().map(|s| s.name.as_str()), Some("[3000]"));
    }

    #[test]
    fn bracketed_keys_keep_their_source_text() {
        let got = outline_of("local t = { ['a b'] = 1, [\"c\"] = 2, [3] = 4, [k] = 5, d = 6 }\n");
        assert_eq!(
            got,
            ["t", "  ['a b']", "  [\"c\"]", "  [3]", "  [k]", "  d"]
        );
    }

    #[test]
    fn string_index_assignment_is_not_a_field() {
        let value = names(&symbols("local M = {}\nM['x'] = 1\n"));
        assert_eq!(value, [("M".to_string(), SymbolKind::OBJECT)]);

        let closure = names(&symbols("M['y'] = function() end\n"));
        assert_eq!(closure, [(" ".to_string(), SymbolKind::FUNCTION)]);
    }

    #[test]
    fn local_kind_follows_the_assigned_value() {
        let got = names(&symbols(
            "local s = 's'\nlocal n = 1\nlocal b = true\nlocal o = {}\nlocal a = {1, 2}\nlocal m = { 1, k = 2 }\nlocal f = function() end\nlocal u = g()\nlocal v = f\nlocal x, y = 1\n",
        ));
        let kinds: Vec<_> = got.iter().map(|(_, kind)| *kind).collect();
        assert_eq!(
            kinds,
            [
                SymbolKind::STRING,
                SymbolKind::NUMBER,
                SymbolKind::BOOLEAN,
                SymbolKind::OBJECT,
                SymbolKind::ARRAY,
                SymbolKind::OBJECT,
                SymbolKind::FUNCTION,
                SymbolKind::VARIABLE,
                SymbolKind::VARIABLE,
                SymbolKind::NUMBER,
                SymbolKind::VARIABLE,
            ]
        );
    }

    #[test]
    fn table_field_and_assignment_kind_follow_the_value() {
        let outline = symbols(
            "local M = { a = 'x', b = false, c = 2, d = {}, e = { 1 }, f = function() end, g = h() }\nM.i = 'y'\nM.j = {}\n",
        );
        let mut flat = Vec::new();
        fn collect(symbols: &[DocumentSymbol], out: &mut Vec<(String, SymbolKind)>) {
            for symbol in symbols {
                out.push((symbol.name.clone(), symbol.kind));
                collect(symbol.children.as_deref().unwrap_or_default(), out);
            }
        }
        collect(&outline, &mut flat);
        let kind = |name: &str| flat.iter().find(|(n, _)| n == name).map(|(_, k)| *k);
        assert_eq!(kind("a"), Some(SymbolKind::STRING));
        assert_eq!(kind("b"), Some(SymbolKind::BOOLEAN));
        assert_eq!(kind("c"), Some(SymbolKind::NUMBER));
        assert_eq!(kind("d"), Some(SymbolKind::OBJECT));
        assert_eq!(kind("e"), Some(SymbolKind::ARRAY));
        assert_eq!(kind("f"), Some(SymbolKind::FUNCTION));
        assert_eq!(kind("g"), Some(SymbolKind::VARIABLE));
        assert_eq!(kind("M.i"), Some(SymbolKind::STRING));
        assert_eq!(kind("M.j"), Some(SymbolKind::OBJECT));
    }
}
