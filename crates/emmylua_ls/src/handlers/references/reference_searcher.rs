//! # reference_searcher — pure Semantic reference search
//!
//! The legacy DbIndex version (module export alias following / ctor references / fuzzy / string references)
//! was moved to `handlers/common/legacy_references.rs` for not-yet-migrated handlers (M3 batch 3+),
//! and will be deleted after the compilation layer is retired.

use std::collections::HashSet;

use emmylua_code_analysis::{
    DeclKind, FileId, LuaMemberKey, LuaType, SemanticDatabase, SemanticId, SemanticModel, TypeDef,
};
use emmylua_parser::{
    LuaAstNode, LuaAstToken, LuaCallExpr, LuaIndexExpr, LuaLiteralToken, LuaNameExpr,
    LuaSyntaxKind, LuaSyntaxToken, LuaTableField,
};
use lsp_types::Location;

use crate::handlers::common::{
    class_fields_of_table_key, decl_reference_ranges, label_reference_ranges,
    member_reference_ranges, type_def_reference_ranges,
};

/// Entry point: token position → list of reference positions.
pub fn search_references(
    model: &SemanticModel<'_>,
    db: &SemanticDatabase,
    token: LuaSyntaxToken,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    // 1. Label references (same-file pure syntax).
    if let Some(label_ranges) = label_reference_ranges(model, &token, include_declaration) {
        return Some(
            label_ranges
                .into_iter()
                .filter_map(|range| location_of(db, model.file_id(), range))
                .collect(),
        );
    }

    // 2. Semantic declaration references (decl / member / typedef).
    if let Some(decl) = model.find_decl(token.clone().into()) {
        let ranges = match &decl {
            SemanticId::Decl(_) => {
                let mut ranges = Vec::new();
                for same in same_name_global_decls(model, &decl) {
                    for range in decl_reference_ranges_with_aliases(db, &same, include_declaration)
                    {
                        push_unique(&mut ranges, range);
                    }
                }
                ranges
            }
            SemanticId::Member(_) => {
                let mut ranges = Vec::new();
                let mut members = same_slot_members(model, &decl);
                for field in class_fields_of_receiver(model, &token) {
                    for member in same_slot_members(model, &field) {
                        if !members.contains(&member) {
                            members.push(member);
                        }
                    }
                }
                for member in &members {
                    for range in
                        member_reference_ranges_with_require(db, member, include_declaration)
                    {
                        push_unique(&mut ranges, range);
                    }
                }
                for range in typed_receiver_uses(db, model, &decl, &members) {
                    push_unique(&mut ranges, range);
                }
                ranges
            }
            SemanticId::TypeDef(_) => match resolve_type_def_of_id(model, &decl) {
                Some(def) => type_def_reference_ranges(db, &def, include_declaration),
                None => Vec::new(),
            },
            _ => Vec::new(),
        };
        return Some(
            ranges
                .into_iter()
                .filter_map(|(file_id, range)| location_of(db, file_id, range))
                .collect(),
        );
    }

    // 3. Field of a receiver that nothing declares and no type explains (`vim.fn.stdpath`, `h.check`):
    // the other accesses of the same field on the same variable.
    if let Some(ranges) = unresolved_member_uses(model, db, &token) {
        return Some(
            ranges
                .into_iter()
                .filter_map(|(file_id, range)| location_of(db, file_id, range))
                .collect(),
        );
    }

    // 3b. A global nothing declares (`before_each`, `describe` supplied by a test runner): every
    // use of the name in the workspace.
    if let Some(ranges) = undeclared_global_uses(model, db, &token) {
        return Some(
            ranges
                .into_iter()
                .filter_map(|(file_id, range)| location_of(db, file_id, range))
                .collect(),
        );
    }

    // 4. String reference index and fuzzy search retired with the old reference index (no Semantic equivalent).
    Some(Vec::new())
}

/// A global declared in several files (`function printf` in each standalone script, `Mod = Mod or {}`
/// in each module) is one variable: the declaration itself followed by its same-named siblings.
/// Any other declaration stands alone.
pub(crate) fn same_name_global_decls(
    model: &SemanticModel<'_>,
    decl: &SemanticId,
) -> Vec<SemanticId> {
    let mut out = vec![decl.clone()];
    let SemanticId::Decl(key) = decl else {
        return out;
    };
    let Some(info) = model
        .file_facts_of(key.file_id)
        .and_then(|facts| facts.decl_by_id(decl))
    else {
        return out;
    };
    if !matches!(info.kind, DeclKind::Global) {
        return out;
    }
    for sibling in model.global_decls_named_for_file(key.file_id, info.name.as_str()) {
        if !out.contains(&sibling) {
            out.push(sibling);
        }
    }
    out
}

/// Every member sharing the owner and name of `member`, itself included.
///
/// `function M.f` in the module and `mod.f = stub` in a test file are separate member identities
/// for one logical field; use sites are attributed to whichever the resolver meets first.
fn same_slot_members(model: &SemanticModel<'_>, member: &SemanticId) -> Vec<SemanticId> {
    let siblings = member_slot(model, member).map(|(owner, name)| {
        let mut owners = vec![owner.clone()];
        owners.extend(model.self_related_owners(&owner));
        owners
            .iter()
            .flat_map(|owner| model.members_of_owner_named(owner, name.as_str()).to_vec())
            .map(|sibling| sibling.id.clone())
            .collect::<Vec<_>>()
    });
    let mut out = vec![member.clone()];
    for sibling in siblings.unwrap_or_default() {
        if !out.contains(&sibling) {
            out.push(sibling);
        }
    }
    out
}

fn member_slot(model: &SemanticModel<'_>, member: &SemanticId) -> Option<(SemanticId, String)> {
    let SemanticId::Member(key) = member else {
        return None;
    };
    let facts = model.file_facts_of(key.file_id)?;
    let fact = facts.member_by_id(member)?;
    let name = model
        .members_of_owner(&fact.owner)
        .iter()
        .find(|candidate| &candidate.id == member)?
        .name
        .to_string();
    Some((fact.owner.clone(), name))
}

/// The class fields (`---@field version? integer`) a key like `state.version` names through the
/// type of its receiver: an assignment to a field of a typed receiver is a use of that field.
fn class_fields_of_receiver(model: &SemanticModel<'_>, token: &LuaSyntaxToken) -> Vec<SemanticId> {
    if let Some(field) = token.parent().and_then(LuaTableField::cast) {
        return class_fields_of_table_key(model, &field);
    }
    let Some(index_expr) = token.parent().and_then(LuaIndexExpr::cast) else {
        return Vec::new();
    };
    let Some(key) = index_expr.get_index_key() else {
        return Vec::new();
    };
    if key.get_range() != Some(token.text_range()) {
        return Vec::new();
    }
    let Some(prefix) = index_expr.get_prefix_expr() else {
        return Vec::new();
    };
    let receiver_type = model.type_of_expr(prefix.get_syntax_id());
    let (LuaType::Ref(type_id) | LuaType::Def(type_id)) = &receiver_type else {
        return Vec::new();
    };
    let Some(class) = model.type_def_of(type_id) else {
        return Vec::new();
    };
    let member_key = LuaMemberKey::Name(key.get_path_part().into());
    // Only the receiver's own `---@field`s: a parent class's field of the same name is another slot.
    model
        .member_infos_with_key_all(&receiver_type, &member_key)
        .into_iter()
        .filter_map(|info| info.id)
        .filter(|id| {
            let SemanticId::Member(member_key) = id else {
                return false;
            };
            model
                .file_facts_of(member_key.file_id)
                .and_then(|facts| facts.member_by_id(id))
                .is_some_and(|fact| fact.owner == class.id)
        })
        .collect()
}

/// Uses of the member whose receiver is only known by type (`local r = M.open(); r:close()`).
///
/// The reference index attributes uses by the syntactic owner, which cannot see through a local
/// initialised from a call. Files whose text never mentions the member name are skipped.
fn typed_receiver_uses(
    db: &SemanticDatabase,
    model: &SemanticModel<'_>,
    member: &SemanticId,
    slot: &[SemanticId],
) -> Vec<(FileId, rowan::TextRange)> {
    let Some((_, name)) = member_slot(model, member) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for file_id in db.file_ids() {
        let Some(document) = db.document(file_id) else {
            continue;
        };
        if !document.get_text().contains(name.as_str()) {
            continue;
        }
        let file_model = SemanticModel::new(db, file_id);
        let Some(chunk) = file_model.chunk() else {
            continue;
        };
        for index_expr in chunk.descendants::<LuaIndexExpr>() {
            let Some(key) = index_expr.get_index_key() else {
                continue;
            };
            if key.get_path_part() != name {
                continue;
            }
            let resolved = file_model
                .resolve_member(&index_expr)
                .and_then(|resolved| resolved.member_id);
            if resolved.is_some_and(|id| slot.contains(&id))
                && let Some(range) = key.get_range()
            {
                push_unique(&mut out, (file_id, range));
            }
        }
        // `set(1, { name = v })`: a key of a literal whose expected class declares the field.
        for table_field in chunk.descendants::<LuaTableField>() {
            let Some(key) = table_field.get_field_key() else {
                continue;
            };
            if key.get_path_part() != name {
                continue;
            }
            if class_fields_of_table_key(&file_model, &table_field)
                .iter()
                .any(|id| slot.contains(id))
                && let Some(range) = key.get_range()
            {
                push_unique(&mut out, (file_id, range));
            }
        }
    }
    out
}

fn unresolved_member_uses(
    model: &SemanticModel<'_>,
    db: &SemanticDatabase,
    token: &LuaSyntaxToken,
) -> Option<Vec<(FileId, rowan::TextRange)>> {
    let index_expr = token.parent()?.ancestors().find_map(LuaIndexExpr::cast)?;
    let key_range = index_expr.get_index_key()?.get_range()?;
    if key_range != token.text_range() {
        return None;
    }
    let key = model.unresolved_member_key(&index_expr)?;
    let mut out = Vec::new();
    for range in db.unresolved_member_reference_ranges(&key) {
        push_unique(&mut out, range);
    }
    push_unique(&mut out, (model.file_id(), key_range));
    Some(out)
}

/// Uses of the undeclared global `token` names, in every file where the name also resolves to nothing.
fn undeclared_global_uses(
    model: &SemanticModel<'_>,
    db: &SemanticDatabase,
    token: &LuaSyntaxToken,
) -> Option<Vec<(FileId, rowan::TextRange)>> {
    let name_expr = token.parent().and_then(LuaNameExpr::cast)?;
    if model.resolve_name(token.text_range().start()).is_some() {
        return None;
    }
    let name = name_expr.get_name_text()?;
    let mut out = Vec::new();
    for file_id in db.file_ids() {
        let file_model = SemanticModel::new(db, file_id);
        let Some(uses) = file_model.name_uses() else {
            continue;
        };
        for name_use in uses.iter().filter(|name_use| name_use.name == name) {
            let range = name_use.syntax.get_range();
            if file_model.resolve_name(range.start()).is_none() {
                push_unique(&mut out, (file_id, range));
            }
        }
    }
    Some(out)
}

/// Type definition identity → TypeDef (after `find_decl` returns `SemanticId::TypeDef`, fetch the definition details).
fn resolve_type_def_of_id(model: &SemanticModel<'_>, id: &SemanticId) -> Option<TypeDef> {
    let SemanticId::TypeDef(key) = id else {
        return None;
    };
    model
        .type_defs_in_scope(key.scope, &key.full_name)
        .iter()
        .find(|def| def.id == *id)
        .cloned()
}

/// `require("mod").field` → member identity in the module file facts.
fn require_index_member(
    model: &SemanticModel<'_>,
    index_expr: &LuaIndexExpr,
) -> Option<SemanticId> {
    model.require_field_member(index_expr)
}

/// Reference ranges for require module path strings (the legacy reference index also counted the
/// `"mod"` in `require("mod").field` as a member alias reference).
fn require_module_literal_ranges(
    db: &SemanticDatabase,
    member: &SemanticId,
) -> Vec<(FileId, rowan::TextRange)> {
    let mut out = Vec::new();
    for file_id in db.file_ids() {
        let model = SemanticModel::new(db, file_id);
        let Some(chunk) = model.chunk() else {
            continue;
        };
        for index_expr in chunk.descendants::<LuaIndexExpr>() {
            if require_index_member(&model, &index_expr).as_ref() != Some(member) {
                continue;
            }
            let Some(prefix) = index_expr.get_prefix_expr() else {
                continue;
            };
            let emmylua_parser::LuaExpr::CallExpr(call) = prefix else {
                continue;
            };
            let Some(arg) = call.get_args_list().and_then(|list| list.get_args().next()) else {
                continue;
            };
            push_unique(&mut out, (file_id, arg.get_range()));
        }
    }
    out
}

/// Member reference ranges + `require("mod").member` usage sites (`resolve_member` does not yet give
/// alias identities for require-module exported members; here they are filled in by module file + member key).
fn member_reference_ranges_with_require(
    db: &SemanticDatabase,
    member: &SemanticId,
    include_declaration: bool,
) -> Vec<(FileId, rowan::TextRange)> {
    let mut out = member_reference_ranges(db, member, include_declaration);
    for file_id in db.file_ids() {
        let model = SemanticModel::new(db, file_id);
        let Some(chunk) = model.chunk() else {
            continue;
        };
        for index_expr in chunk.descendants::<LuaIndexExpr>() {
            if require_index_member(&model, &index_expr).as_ref() == Some(member)
                && let Some(key) = index_expr.get_index_key()
                && let Some(range) = key.get_range()
            {
                push_unique(&mut out, (file_id, range));
            }
        }
    }
    out
}

/// Declaration references + module export alias chains:
/// Declarations in `local function flush() ...; return { flush = flush }` are included, and cross-file
/// usage sites like `local f = require("mod").flush; f()` must also be counted.
fn decl_reference_ranges_with_aliases(
    db: &SemanticDatabase,
    decl: &SemanticId,
    include_declaration: bool,
) -> Vec<(FileId, rowan::TextRange)> {
    let mut out = decl_reference_ranges(db, decl, include_declaration);
    let mut visited_members: HashSet<SemanticId> = HashSet::new();
    let mut visited_decls: HashSet<SemanticId> = HashSet::new();
    let mut member_work = alias_members_of_decl(db, decl);
    let mut decl_work = Vec::new();
    let mut module_alias_work = module_alias_decls_of_decl(db, decl);

    while !member_work.is_empty() || !decl_work.is_empty() || !module_alias_work.is_empty() {
        for (module_decl, literal_ranges) in module_alias_work.drain(..) {
            if visited_decls.insert(module_decl.clone()) {
                out.extend(decl_reference_ranges(db, &module_decl, include_declaration));
            }
            out.extend(literal_ranges);
            member_work.extend(alias_members_of_decl(db, &module_decl));
        }
        for member in member_work.drain(..) {
            if !visited_members.insert(member.clone()) {
                continue;
            }
            out.extend(member_reference_ranges_with_require(
                db,
                &member,
                include_declaration,
            ));
            out.extend(require_module_literal_ranges(db, &member));
            decl_work.extend(decls_aliased_to_member(db, &member));
        }
        for alias_decl in decl_work.drain(..) {
            if !visited_decls.insert(alias_decl.clone()) {
                continue;
            }
            out.extend(decl_reference_ranges(db, &alias_decl, include_declaration));
            member_work.extend(alias_members_of_decl(db, &alias_decl));
        }
    }

    out.sort_by_key(|(file_id, range)| {
        (file_id.id, u32::from(range.start()), u32::from(range.end()))
    });
    out.dedup();
    out
}

/// `return init` module export: other files alias the declaration to the target via `local f = require("mod")`.
fn module_alias_decls_of_decl(
    db: &SemanticDatabase,
    decl: &SemanticId,
) -> Vec<(SemanticId, Vec<(FileId, rowan::TextRange)>)> {
    let mut out = Vec::new();
    let mut module_files = Vec::new();
    for file_id in db.file_ids() {
        let model = SemanticModel::new(db, file_id);
        let Some(exports) = model.file_exports(file_id) else {
            continue;
        };
        if matches!(
            &exports.module,
            Some(emmylua_code_analysis::ModuleExport::Decl {
                decl: exported,
                ..
            }) if exported == decl
        ) {
            module_files.push(file_id);
        }
    }

    for module_file in module_files {
        let Some(module_name) = db.module_name_of(module_file) else {
            continue;
        };
        for file_id in db.file_ids() {
            let model = SemanticModel::new(db, file_id);
            let Some(decls) = model.decls() else {
                continue;
            };
            for alias_decl in decls.iter() {
                let Some(value_syntax) = alias_decl.value_expr_syntax else {
                    continue;
                };
                let Some(require_name) = require_module_name_at(&model, value_syntax) else {
                    continue;
                };
                if require_name == module_name {
                    out.push((alias_decl.id.clone(), Vec::new()));
                }
            }
        }
    }
    out
}

fn require_module_name_at(
    model: &SemanticModel<'_>,
    call_syntax: emmylua_parser::LuaSyntaxId,
) -> Option<String> {
    let chunk = model.chunk()?;
    let call = chunk
        .descendants::<LuaCallExpr>()
        .find(|call| call.get_syntax_id() == call_syntax)?;
    if !call.is_require() {
        return None;
    }
    let arg = call.get_args_list()?.get_args().next()?;
    let literal = arg
        .syntax()
        .descendants_with_tokens()
        .filter_map(|it| it.into_token())
        .find_map(LuaLiteralToken::cast)?;
    match literal {
        LuaLiteralToken::String(string) => Some(string.get_value()),
        _ => None,
    }
}

/// Which member aliases reference a declaration (the `flush` member in `export.flush = flush`).
fn alias_members_of_decl(db: &SemanticDatabase, decl: &SemanticId) -> Vec<SemanticId> {
    let mut out = Vec::new();
    for file_id in db.file_ids() {
        let model = SemanticModel::new(db, file_id);
        let Some(members) = model.members() else {
            continue;
        };
        for member in members.iter() {
            let Some(value_syntax) = member.value_syntax else {
                continue;
            };
            // `m.f = make()` stores what `make` returns, not `make` itself.
            if value_syntax.get_kind() == LuaSyntaxKind::CallExpr {
                continue;
            }
            let resolved = model.resolve_name(value_syntax.get_range().start());
            if resolved == Some(decl.clone()) {
                out.push(member.id.clone());
            }
        }
    }
    out
}

/// Which declarations are initialized from a member alias (the `f` in `local f = require("mod").flush`).
fn decls_aliased_to_member(db: &SemanticDatabase, member: &SemanticId) -> Vec<SemanticId> {
    let mut out = Vec::new();
    for file_id in db.file_ids() {
        let model = SemanticModel::new(db, file_id);
        let Some(decls) = model.decls() else {
            continue;
        };
        let Some(chunk) = model.chunk() else {
            continue;
        };
        for decl in decls.iter() {
            let Some(value_syntax) = decl.value_expr_syntax else {
                continue;
            };
            for index_expr in chunk.descendants::<LuaIndexExpr>() {
                if index_expr.get_syntax_id() != value_syntax {
                    continue;
                }
                let resolved = model
                    .resolve_member(&index_expr)
                    .and_then(|resolved| resolved.member_id)
                    .or_else(|| require_index_member(&model, &index_expr));
                if resolved.as_ref() == Some(member) {
                    out.push(decl.id.clone());
                }
                break;
            }
        }
    }
    out
}

fn push_unique(out: &mut Vec<(FileId, rowan::TextRange)>, item: (FileId, rowan::TextRange)) {
    if !out.contains(&item) {
        out.push(item);
    }
}

pub(crate) fn location_of(
    db: &SemanticDatabase,
    file_id: FileId,
    range: rowan::TextRange,
) -> Option<Location> {
    let document = db.document(file_id)?;
    let uri = document.get_uri()?;
    Some(Location {
        uri,
        range: document.to_lsp_range(range)?,
    })
}
