//! The class a table literal is expected to be, read from where the literal is written: the type
//! of the parameter it is passed to (`set(1, { hl_group = "A" })`) or of the field of an enclosing
//! literal. LuaLS counts the keys of such a literal as uses of that class's fields.

use emmylua_code_analysis::{LuaMemberKey, LuaType, SemanticId, SemanticModel};
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaCallArgList, LuaCallExpr, LuaIndexKey, LuaTableExpr,
    LuaTableField,
};

/// The named type a table literal stands for, with `nil` stripped (`Opts?` -> `Opts`).
pub fn expected_class_of_table(model: &SemanticModel<'_>, table: &LuaTableExpr) -> Option<LuaType> {
    if let Some(field) = table.syntax().parent().and_then(LuaTableField::cast) {
        // `{ outer = { key = v } }`: the type of `outer` in the enclosing literal's class.
        let parent_table = field.get_parent::<LuaTableExpr>()?;
        let parent_class = expected_class_of_table(model, &parent_table)?;
        let key = field.get_field_key()?;
        let member_key = LuaMemberKey::Name(key.get_path_part().into());
        let member = model
            .member_infos_with_key_all(&parent_class, &member_key)
            .into_iter()
            .next()?;
        return named_type(&member.typ);
    }
    // `self.client_state[id] = { key = v }`: the declared type of the assigned target.
    if let Some(assign) = table.syntax().parent().and_then(LuaAssignStat::cast) {
        let (vars, values) = assign.get_var_and_expr_list();
        let index = values
            .iter()
            .position(|value| value.get_syntax_id() == table.get_syntax_id())?;
        let var = vars.get(index)?;
        return named_type(&model.type_of_expr(var.to_expr().get_syntax_id()));
    }
    let call = table
        .syntax()
        .parent()
        .and_then(LuaCallArgList::cast)?
        .get_parent::<LuaCallExpr>()?;
    let arg_index = call
        .get_args_list()?
        .get_args()
        .position(|arg| arg.get_syntax_id() == table.get_syntax_id())?;
    let prefix = call.get_prefix_expr()?;
    for func in model.callable_candidates_for_expr(&prefix) {
        let mut index = arg_index;
        if call.is_colon_call() && !func.is_colon_define() {
            index += 1;
        }
        if let Some(Some(ty)) = func.get_params().get(index).map(|(_, ty)| ty.as_ref())
            && let Some(class) = named_type(ty)
        {
            return Some(class);
        }
    }
    None
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
    let (LuaIndexKey::Name(_) | LuaIndexKey::String(_)) = key else {
        return Vec::new();
    };
    let Some(class_type) = expected_class_of_table(model, &table) else {
        return Vec::new();
    };
    let (LuaType::Ref(type_id) | LuaType::Def(type_id)) = &class_type else {
        return Vec::new();
    };
    let Some(class) = model.type_def_of(type_id) else {
        return Vec::new();
    };
    let member_key = LuaMemberKey::Name(key.get_path_part().into());
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
        })
        .collect()
}

fn named_type(ty: &LuaType) -> Option<LuaType> {
    match ty {
        LuaType::Ref(_) | LuaType::Def(_) | LuaType::Generic(_) => Some(ty.clone()),
        LuaType::Union(union) => union.into_vec().iter().find_map(named_type),
        _ => None,
    }
}
