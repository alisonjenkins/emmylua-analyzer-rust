use std::collections::BTreeMap;

use emmylua_code_analysis::{LuaType, Member, SemanticId, SemanticModel};
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaExpr, LuaIndexExpr, LuaIndexKey, LuaNameExpr, LuaVarExpr,
};

use super::render::humanize;

/// Fields listed before the shape is cut off; keeps the hover of a heavily used parameter readable.
const MAX_SHAPE_FIELDS: usize = 24;
/// Distinct assigned types listed for one field before the rest are dropped.
const MAX_FIELD_TYPES: usize = 3;

/// Renders the table shape of an untyped parameter or local from how its body uses it:
/// `x.f`, `x["f"]`, `x:f()` add the field `f`, and `x.f = v` types it as the widened type of `v`.
/// `None` when the declaration is not used as a table anywhere.
pub fn render_usage_shape(model: &SemanticModel<'_>, decl: &SemanticId) -> Option<String> {
    let tree = model.syntax_tree()?;
    let root = tree.get_red_root();
    let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for reference in model.decl_references(decl) {
        let Some(name_expr) = reference
            .to_node_from_root(&root)
            .and_then(LuaNameExpr::cast)
        else {
            continue;
        };
        let Some(index_expr) = name_expr
            .syntax()
            .parent()
            .and_then(LuaIndexExpr::cast)
            .filter(|index| {
                index
                    .get_prefix_expr()
                    .is_some_and(|prefix| prefix.syntax() == name_expr.syntax())
            })
        else {
            continue;
        };
        record_field_use(model, &mut fields, &index_expr);
    }
    format_shape(fields)
}

/// Same shape for a member (`self.params`) from the fields its holders read, write and call elsewhere in the file:
/// `self.params.acls = 1`, `self.params.group`, `self.params:get()`.
pub fn render_member_usage_shape(model: &SemanticModel<'_>, member: &Member) -> Option<String> {
    let name = member.key.to_path();
    let tree = model.syntax_tree()?;
    let root = tree.get_red_root();
    let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for node in root.descendants() {
        let Some(index_expr) = LuaIndexExpr::cast(node) else {
            continue;
        };
        let Some(LuaExpr::IndexExpr(holder)) = index_expr.get_prefix_expr() else {
            continue;
        };
        if holder
            .get_index_key()
            .is_none_or(|key| key.get_path_part() != name)
        {
            continue;
        }
        let Some(resolved) = model.resolve_member(&holder) else {
            continue;
        };
        let (Some(id), Some(file_id)) = (resolved.member_id, resolved.file_id) else {
            continue;
        };
        let same_member = model
            .file_facts_of(file_id)
            .and_then(|facts| facts.member_by_id(&id))
            .is_some_and(|found| found.owner == member.owner && found.key == member.key);
        if same_member {
            record_field_use(model, &mut fields, &index_expr);
        }
    }
    format_shape(fields)
}

fn record_field_use(
    model: &SemanticModel<'_>,
    fields: &mut BTreeMap<String, Vec<String>>,
    index_expr: &LuaIndexExpr,
) {
    let Some(key) = field_key(index_expr) else {
        return;
    };
    let types = fields.entry(key).or_default();
    if let Some(assigned) = assigned_type(model, index_expr)
        && !types.contains(&assigned)
        && types.len() < MAX_FIELD_TYPES
    {
        types.push(assigned);
    }
}

fn format_shape(fields: BTreeMap<String, Vec<String>>) -> Option<String> {
    if fields.is_empty() {
        return None;
    }
    let total = fields.len();
    let mut parts: Vec<String> = fields
        .into_iter()
        .take(MAX_SHAPE_FIELDS)
        .map(|(key, types)| {
            let ty = if types.is_empty() {
                "unknown".to_string()
            } else {
                types.join("|")
            };
            format!("{key}: {ty}")
        })
        .collect();
    if total > MAX_SHAPE_FIELDS {
        parts.push("...".to_string());
    }
    Some(format!("{{ {} }}", parts.join(", ")))
}

fn field_key(index_expr: &LuaIndexExpr) -> Option<String> {
    match index_expr.get_index_key()? {
        LuaIndexKey::Name(name) => Some(name.get_name_text().to_string()),
        LuaIndexKey::String(string) => Some(format!("[\"{}\"]", string.get_value())),
        _ => None,
    }
}

/// Type of the value assigned to `index_expr` when it is the target of `a.f = v`.
fn assigned_type(model: &SemanticModel<'_>, index_expr: &LuaIndexExpr) -> Option<String> {
    let assign = index_expr.syntax().parent().and_then(LuaAssignStat::cast)?;
    let (vars, values) = assign.get_var_and_expr_list();
    let position = vars.iter().position(|var| match var {
        LuaVarExpr::IndexExpr(candidate) => candidate.syntax() == index_expr.syntax(),
        _ => false,
    })?;
    let value: &LuaExpr = values.get(position)?;
    let ty = model.type_of_expr(value.get_syntax_id());
    Some(shape_field_type(model, &ty))
}

/// Widened rendering of an assigned value: literals lose their value and tables and functions collapse.
fn shape_field_type(model: &SemanticModel<'_>, ty: &LuaType) -> String {
    match ty {
        LuaType::IntegerConst(_) | LuaType::DocIntegerConst(_) => "integer".to_string(),
        LuaType::FloatConst(_) => "number".to_string(),
        LuaType::StringConst(_) | LuaType::DocStringConst(_) => "string".to_string(),
        LuaType::BooleanConst(_) | LuaType::DocBooleanConst(_) => "boolean".to_string(),
        LuaType::TableConst(_) | LuaType::Object(_) | LuaType::Table => "table".to_string(),
        LuaType::Signature(_) | LuaType::DocFunction(_) => "function".to_string(),
        LuaType::Unknown => "unknown".to_string(),
        _ => humanize(model, ty),
    }
}
