//! # Value alias origin tracing
//!
//! For simple aliases like `local b = a` / `local f = t.func`, hover and goto need
//! to trace back to the real declaration (function definition / member definition), so the original signature and comments can be shown.

use std::collections::HashSet;

use emmylua_code_analysis::{LuaType, ModuleExport, SemanticId, SemanticModel};
use emmylua_parser::{
    LuaAstNode, LuaAstToken, LuaClosureExpr, LuaExpr, LuaIndexExpr, LuaReturnStat,
};

/// Follow the value initialization chain of a local variable / member to its final declared identity.
///
/// - `local a = b` → the declaration of `b`
/// - `local f = t.func` → the member declaration of `t.func`
/// - `function M:f() end` / `local function f() end` return themselves directly
pub fn resolve_alias_origin(model: &SemanticModel<'_>, decl: &SemanticId) -> Option<SemanticId> {
    let mut visited = HashSet::new();
    let mut current = decl.clone();
    loop {
        if !visited.insert(current.clone()) {
            return None;
        }
        match &current {
            SemanticId::Decl(key) => {
                let file_model = model.model_for(key.file_id);
                let facts = file_model.file_facts()?;
                let decl_info = facts.decl_by_id(&current)?;
                let Some(value_syntax) = decl_info.value_expr_syntax else {
                    // Parameters/uninitialized locals have no traceable value expression: the declaration itself is the final origin.
                    return Some(current);
                };
                let tree = file_model.syntax_tree()?;
                let node = value_syntax.to_node_from_root(&tree.get_red_root())?;
                let expr = LuaExpr::cast(node)?;
                match expr {
                    LuaExpr::ClosureExpr(_) => return Some(current),
                    LuaExpr::NameExpr(name_expr) => {
                        current = file_model.resolve_name(name_expr.get_position())?;
                    }
                    LuaExpr::IndexExpr(index_expr) => {
                        current = file_model
                            .resolve_member(&index_expr)
                            .and_then(|resolved| resolved.member_id)
                            .or_else(|| resolve_require_index_member(&file_model, &index_expr))?;
                    }
                    LuaExpr::CallExpr(call) if call.is_require() => {
                        let arg = call.get_args_list()?.get_args().next()?;
                        let module_name = string_literal_of(&arg)?;
                        let module_file = file_model.module_file_of(&module_name)?;
                        let module_facts = file_model.file_facts_of(module_file)?;
                        match &module_facts.module_export {
                            ModuleExport::Decl { decl, .. } => {
                                current = decl.clone();
                            }
                            ModuleExport::Global { name } => {
                                current = file_model
                                    .global_decl(name.as_str())
                                    .unwrap_or_else(|| SemanticId::name(name.clone()));
                            }
                            _ => return None,
                        }
                    }
                    _ => return None,
                }
            }
            SemanticId::Member(key) => {
                let file_model = model.model_for(key.file_id);
                let facts = file_model.file_facts()?;
                let member = facts.member_by_id(&current)?;
                // Method definitions/function-valued fields are themselves the final definition.
                if member.is_method {
                    return Some(current);
                }
                let Some(value_syntax) = member.value_syntax else {
                    return Some(current);
                };
                let tree = file_model.syntax_tree()?;
                let node = value_syntax.to_node_from_root(&tree.get_red_root())?;
                let Some(expr) = LuaExpr::cast(node) else {
                    return Some(current);
                };
                match expr {
                    LuaExpr::ClosureExpr(_) => return Some(current),
                    LuaExpr::NameExpr(name_expr) => {
                        current = file_model.resolve_name(name_expr.get_position())?;
                    }
                    LuaExpr::IndexExpr(index_expr) => {
                        current = file_model
                            .resolve_member(&index_expr)
                            .and_then(|resolved| resolved.member_id)
                            .or_else(|| resolve_require_index_member(&file_model, &index_expr))?;
                    }
                    _ => return Some(current),
                }
            }
            other => return Some(other.clone()),
        }
    }
}

/// Where the function a declaration or field stands for is defined, when it is not the declaration
/// itself: the function a field/alias is set to, and the closure a called function returns.
/// LuaLS lists these beside the declaration in both go-to-definition and find-references.
pub fn value_origin_sites(
    model: &SemanticModel<'_>,
    decl: &SemanticId,
) -> Vec<(emmylua_code_analysis::FileId, rowan::TextRange)> {
    let mut sites = Vec::new();
    if let Some(origin) = resolve_alias_origin(model, decl)
        && origin != *decl
        && is_function_valued(model, &origin)
    {
        match &origin {
            SemanticId::Decl(key) => sites.push((key.file_id, key.name_range)),
            SemanticId::Member(key) => sites.push((key.file_id, key.key_range)),
            _ => {}
        }
    }
    sites.extend(call_value_function(model, decl));
    sites
}

/// The function a declaration's value evaluates to when that value is a call (`x = make()`,
/// `{ key = make() }`): the closure the callee returns. LuaLS lists it beside the assignment.
///
/// Follows `resolve_alias_origin` first so `local g = cfg.on_init` reaches the same closure.
pub fn call_value_function(
    model: &SemanticModel<'_>,
    decl: &SemanticId,
) -> Option<(emmylua_code_analysis::FileId, rowan::TextRange)> {
    let origin = resolve_alias_origin(model, decl);
    let candidates = std::iter::once(decl.clone()).chain(origin.filter(|origin| origin != decl));
    for id in candidates {
        let Some(file_id) = semantic_id_file(&id) else {
            continue;
        };
        let file_model = model.model_for(file_id);
        let Some(facts) = file_model.file_facts() else {
            continue;
        };
        let value_syntax = match &id {
            SemanticId::Decl(_) => facts.decl_by_id(&id).and_then(|d| d.value_expr_syntax),
            SemanticId::Member(_) => facts.member_by_id(&id).and_then(|m| m.value_syntax),
            _ => None,
        };
        let Some(value_syntax) = value_syntax else {
            continue;
        };
        let Some(tree) = file_model.syntax_tree() else {
            continue;
        };
        let Some(LuaExpr::CallExpr(call)) = value_syntax
            .to_node_from_root(&tree.get_red_root())
            .and_then(LuaExpr::cast)
        else {
            continue;
        };
        if call.is_require() {
            continue;
        }
        if let Some(found) = returned_function(&file_model, &call) {
            return Some(found);
        }
    }
    None
}

/// The closure a called function returns: `return function() end` or `return local_fn`.
///
/// The call's inferred type is a bare `Function` for these, so the callee's `return` is read directly.
fn returned_function(
    file_model: &SemanticModel<'_>,
    call: &emmylua_parser::LuaCallExpr,
) -> Option<(emmylua_code_analysis::FileId, rowan::TextRange)> {
    let callee = match call.get_prefix_expr()? {
        LuaExpr::NameExpr(name) => file_model.resolve_name(name.get_position())?,
        LuaExpr::IndexExpr(index) => file_model.resolve_member(&index)?.member_id?,
        _ => return None,
    };
    let callee = resolve_alias_origin(file_model, &callee).unwrap_or(callee);
    let (callee_model, callee_closure) = closure_of(file_model, &callee)?;
    let callee_chunk_model = callee_model;
    // A declared `@return` is the answer LuaLS gives, not the closure in the body.
    if callee_chunk_model
        .file_facts()
        .and_then(|facts| facts.signature_by_closure(callee_closure.get_syntax_id()))
        .and_then(|signature| signature.docs.as_ref())
        .is_some_and(|docs| !docs.returns.is_empty())
    {
        return None;
    }
    for ret in callee_closure.descendants::<LuaReturnStat>() {
        let owner = ret.syntax().ancestors().find_map(LuaClosureExpr::cast);
        if owner.as_ref() != Some(&callee_closure) {
            continue;
        }
        match ret.get_expr_list().next() {
            Some(LuaExpr::ClosureExpr(closure)) => {
                return Some((callee_chunk_model.file_id(), closure.get_range()));
            }
            Some(LuaExpr::NameExpr(name)) => {
                let id = callee_chunk_model.resolve_name(name.get_position())?;
                let (closure_model, closure) = closure_of(&callee_chunk_model, &id)?;
                return Some((closure_model.file_id(), closure.get_range()));
            }
            _ => {}
        }
    }
    None
}

/// Whether a declaration stands for a function: a function expression, or a field/variable typed as one.
pub fn is_function_valued(model: &SemanticModel<'_>, id: &SemanticId) -> bool {
    if closure_of(model, id).is_some() {
        return true;
    }
    let Some(file_id) = semantic_id_file(id) else {
        return false;
    };
    let file_model = model.model_for(file_id);
    let ty = match id {
        SemanticId::Decl(_) => file_model.type_of_decl(id),
        SemanticId::Member(_) => file_model.type_of_member(id),
        _ => None,
    };
    matches!(
        ty,
        Some(LuaType::DocFunction(_) | LuaType::Signature(_) | LuaType::Function)
    )
}

/// The function expression a declaration is initialised with.
pub fn closure_of<'db>(
    model: &SemanticModel<'db>,
    id: &SemanticId,
) -> Option<(SemanticModel<'db>, LuaClosureExpr)> {
    let file_model = model.model_for(semantic_id_file(id)?);
    let facts = file_model.file_facts()?;
    let value_syntax = match id {
        SemanticId::Decl(_) => facts.decl_by_id(id)?.value_expr_syntax?,
        SemanticId::Member(_) => facts.member_by_id(id)?.value_syntax?,
        _ => return None,
    };
    let tree = file_model.syntax_tree()?;
    let closure = value_syntax
        .to_node_from_root(&tree.get_red_root())
        .and_then(LuaClosureExpr::cast)?;
    Some((file_model, closure))
}

/// Resolves a direct `require("mod").field` member expression. When `resolve_member`
/// does not recognize a require call prefix, this looks up the member via the module export owner.
fn resolve_require_index_member(
    model: &SemanticModel<'_>,
    index_expr: &LuaIndexExpr,
) -> Option<SemanticId> {
    let LuaExpr::CallExpr(call) = index_expr.get_prefix_expr()? else {
        return None;
    };
    if !call.is_require() {
        return None;
    }
    let arg = call.get_args_list()?.get_args().next()?;
    let module_name = string_literal_of(&arg)?;
    let module_file = model.module_file_of(&module_name)?;
    let module_facts = model.file_facts_of(module_file)?;
    let owner = match &module_facts.module_export {
        ModuleExport::Decl { decl, .. } => decl.clone(),
        ModuleExport::Expr { value_syntax } => {
            SemanticId::member(module_file, value_syntax.get_range())
        }
        _ => return None,
    };
    let key = index_expr.get_index_key()?.get_path_part().to_string();
    let module_model = model.model_for(module_file);
    module_model
        .members_of_owner(&owner)
        .into_iter()
        .find(|member| member.name.as_str() == key)
        .map(|member| member.id.clone())
}

/// File containing the semantic id (returns `None` when a TypeDef Global/Internal scope has no single file).
fn string_literal_of(expr: &LuaExpr) -> Option<String> {
    let token = expr
        .syntax()
        .descendants_with_tokens()
        .filter_map(|it| it.into_token())
        .find_map(emmylua_parser::LuaLiteralToken::cast)?;
    match token {
        emmylua_parser::LuaLiteralToken::String(string) => Some(string.get_value()),
        _ => None,
    }
}

pub fn semantic_id_file(id: &SemanticId) -> Option<emmylua_code_analysis::FileId> {
    match id {
        SemanticId::Decl(key) => Some(key.file_id),
        SemanticId::Member(key) => Some(key.file_id),
        SemanticId::TypeDef(key) => match key.scope {
            emmylua_code_analysis::TypeScope::File(file_id) => Some(file_id),
            _ => None,
        },
        _ => None,
    }
}
