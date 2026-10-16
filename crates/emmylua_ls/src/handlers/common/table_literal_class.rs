//! The class a table literal is expected to be, read from where the literal is written: the type
//! of the parameter it is passed to (`set(1, { hl_group = "A" })`), of the local it initialises or of
//! the field of an enclosing literal. LuaLS counts the keys of such a literal as uses of that class's fields.

use emmylua_code_analysis::{LuaMemberKey, LuaType, SemanticId, SemanticModel};
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaAstToken, LuaCallArgList, LuaCallExpr, LuaExpr, LuaIndexKey,
    LuaLocalStat, LuaTableExpr, LuaTableField,
};

/// The named types a table literal stands for, with `nil` stripped (`Opts?` -> `Opts`). A union
/// (`TextEdit|InsertReplaceEdit`) gives every class in it.
pub fn expected_classes_of_table(model: &SemanticModel<'_>, table: &LuaTableExpr) -> Vec<LuaType> {
    if let Some(field) = table.syntax().parent().and_then(LuaTableField::cast) {
        // `{ outer = { key = v } }`: the type of `outer` in the enclosing literal's class.
        let (Some(parent_table), Some(key)) =
            (field.get_parent::<LuaTableExpr>(), field.get_field_key())
        else {
            return Vec::new();
        };
        let member_key = LuaMemberKey::Name(key.get_path_part().into());
        let mut out = Vec::new();
        for parent_class in expected_classes_of_table(model, &parent_table) {
            for member in model.member_infos_with_key_all(&parent_class, &member_key) {
                push_named_types(&member.typ, &mut out);
            }
        }
        return out;
    }
    let mut out = Vec::new();
    // `---@type Opts local o = { key = v }`: the declared type of the local.
    if let Some(stat) = table.syntax().parent().and_then(LuaLocalStat::cast) {
        if let Some(name) = stat.get_local_name_by_value(LuaExpr::TableExpr(table.clone()))
            && let Some(token) = name.get_name_token()
            && let Some(decl) = model.decl_by_offset(token.get_position())
            && let Some(ty) = model.type_of_decl(&decl)
        {
            push_named_types(&ty, &mut out);
        }
        return out;
    }
    // `self.client_state[id] = { key = v }`: the declared type of the assigned target.
    if let Some(assign) = table.syntax().parent().and_then(LuaAssignStat::cast) {
        let (vars, values) = assign.get_var_and_expr_list();
        if let Some(index) = values
            .iter()
            .position(|value| value.get_syntax_id() == table.get_syntax_id())
            && let Some(var) = vars.get(index)
        {
            push_named_types(&model.type_of_expr(var.to_expr().get_syntax_id()), &mut out);
        }
        return out;
    }
    let Some(call) = table
        .syntax()
        .parent()
        .and_then(LuaCallArgList::cast)
        .and_then(|args| args.get_parent::<LuaCallExpr>())
    else {
        return out;
    };
    let Some(arg_index) = call.get_args_list().and_then(|args| {
        args.get_args()
            .position(|arg| arg.get_syntax_id() == table.get_syntax_id())
    }) else {
        return out;
    };
    let Some(prefix) = call.get_prefix_expr() else {
        return out;
    };
    for func in model.callable_candidates_for_expr(&prefix) {
        let mut index = arg_index;
        if call.is_colon_call() && !func.is_colon_define() {
            index += 1;
        }
        if let Some(Some(ty)) = func.get_params().get(index).map(|(_, ty)| ty.as_ref()) {
            push_named_types(ty, &mut out);
            if !out.is_empty() {
                break;
            }
        }
    }
    out
}

/// The class fields (`---@field`) a key of a table literal names through the literal's expected
/// class, declared by that class itself (a parent class's field of the same name is another slot).
pub fn class_fields_of_table_key(
    model: &SemanticModel<'_>,
    field: &LuaTableField,
) -> Vec<SemanticId> {
    let (Some(table), Some(key)) = (field.get_parent::<LuaTableExpr>(), field.get_field_key())
    else {
        return Vec::new();
    };
    let LuaIndexKey::Name(_) = key else {
        return Vec::new();
    };
    let member_key = LuaMemberKey::Name(key.get_path_part().into());
    let mut out = Vec::new();
    for class_type in expected_classes_of_table(model, &table) {
        let (LuaType::Ref(type_id) | LuaType::Def(type_id)) = &class_type else {
            continue;
        };
        let Some(class) = model.type_def_of(type_id) else {
            continue;
        };
        out.extend(
            model
                .member_infos_with_key_all(&class_type, &member_key)
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
                }),
        );
    }
    out
}

fn push_named_types(ty: &LuaType, out: &mut Vec<LuaType>) {
    match ty {
        LuaType::Ref(_) | LuaType::Def(_) | LuaType::Generic(_) => {
            if !out.contains(ty) {
                out.push(ty.clone());
            }
        }
        LuaType::Union(union) => {
            for component in union.into_vec() {
                push_named_types(&component, out);
            }
        }
        _ => {}
    }
}
