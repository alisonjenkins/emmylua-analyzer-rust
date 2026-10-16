//! The class a table literal is expected to be, read from where the literal is written: the type
//! of the parameter it is passed to (`set(1, { hl_group = "A" })`), of the local it initialises or of
//! the field of an enclosing literal. LuaLS counts the keys of such a literal as uses of that class's fields.

use emmylua_code_analysis::{LuaMemberKey, LuaType, SemanticId, SemanticModel};
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaAstToken, LuaCallArgList, LuaCallExpr, LuaExpr, LuaIndexExpr,
    LuaIndexKey, LuaLocalStat, LuaTableExpr, LuaTableField,
};

/// The classes a table literal stands for, with `nil` stripped (`Opts?` -> `Opts`). A union
/// (`TextEdit|InsertReplaceEdit`) gives every class in it.
pub fn expected_classes_of_table(model: &SemanticModel<'_>, table: &LuaTableExpr) -> Vec<LuaType> {
    let mut types = expected_types_of_table(model, table);
    types.retain(is_class_type);
    types
}

/// Classes plus the containers (`Elem[]`, `table<string, Elem>`) a literal is declared as: the
/// literals inside a container are its element type.
fn expected_types_of_table(model: &SemanticModel<'_>, table: &LuaTableExpr) -> Vec<LuaType> {
    if let Some(field) = table.syntax().parent().and_then(LuaTableField::cast) {
        // `{ outer = { key = v } }`: the type of `outer` in the enclosing literal's class.
        let (Some(parent_table), Some(key)) =
            (field.get_parent::<LuaTableExpr>(), field.get_field_key())
        else {
            return Vec::new();
        };
        let member_key = matches!(key, LuaIndexKey::Name(_) | LuaIndexKey::String(_))
            .then(|| LuaMemberKey::Name(key.get_path_part().into()));
        let mut out = Vec::new();
        for parent in expected_types_of_table(model, &parent_table) {
            if let Some(element) = container_element_type(&parent) {
                push_named_types(&element, &mut out);
            } else if let Some(member_key) = &member_key {
                for member in model.member_infos_with_key_all(&parent, member_key) {
                    push_named_types(&member.typ, &mut out);
                }
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
            let target = var.to_expr();
            push_named_types(&model.type_of_expr(target.get_syntax_id()), &mut out);
            if let LuaExpr::IndexExpr(index_expr) = &target {
                push_declared_field_types(model, index_expr, &mut out);
            }
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

/// The types the receiver's class declares for `receiver.key`, whatever the assignment gives it.
fn push_declared_field_types(
    model: &SemanticModel<'_>,
    index_expr: &LuaIndexExpr,
    out: &mut Vec<LuaType>,
) {
    let (Some(prefix), Some(key)) = (index_expr.get_prefix_expr(), index_expr.get_index_key())
    else {
        return;
    };
    let member_key = LuaMemberKey::Name(key.get_path_part().into());
    let mut receivers = Vec::new();
    if let LuaExpr::NameExpr(name_expr) = &prefix
        && let Some(decl) = model.resolve_name(name_expr.get_position())
        && let Some(ty) = model.type_of_decl(&decl)
    {
        push_named_types(&ty, &mut receivers);
    }
    push_named_types(&model.type_of_expr(prefix.get_syntax_id()), &mut receivers);
    for receiver in receivers.iter().filter(|ty| is_class_type(ty)) {
        for member in model.member_infos_with_key_all(receiver, &member_key) {
            push_named_types(&member.typ, out);
        }
    }
}

fn is_class_type(ty: &LuaType) -> bool {
    match ty {
        LuaType::Ref(_) | LuaType::Def(_) => true,
        LuaType::Generic(generic) => generic.get_base_type_id_ref().get_name() != "table",
        _ => false,
    }
}

/// The type of the values of `Elem[]` and `table<K, Elem>`.
fn container_element_type(ty: &LuaType) -> Option<LuaType> {
    match ty {
        LuaType::Array(array) => Some(array.get_base().clone()),
        LuaType::TableGeneric(params) => params.get(1).cloned(),
        LuaType::Generic(generic) if generic.get_base_type_id_ref().get_name() == "table" => {
            generic.get_params().get(1).cloned()
        }
        _ => None,
    }
}

fn push_named_types(ty: &LuaType, out: &mut Vec<LuaType>) {
    match ty {
        LuaType::Ref(_)
        | LuaType::Def(_)
        | LuaType::Generic(_)
        | LuaType::Array(_)
        | LuaType::TableGeneric(_) => {
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
