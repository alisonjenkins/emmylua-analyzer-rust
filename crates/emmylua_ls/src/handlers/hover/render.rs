use emmylua_code_analysis::{LuaType, SemanticModel, TypeDefKind, humanize_semantic_type_hover};

pub fn humanize(model: &SemanticModel<'_>, ty: &LuaType) -> String {
    humanize_semantic_type_hover(model, ty)
}

/// Declaration-header rendering. LuaLS writes an optional whose only non-nil member is a class
/// with a dotted name as `(wk.Foo)?`; plain names, arrays and generics stay bare (`Foo?`).
/// Return rows and class field bodies are not headers and keep `humanize`.
pub fn humanize_header(model: &SemanticModel<'_>, ty: &LuaType) -> String {
    if let LuaType::Union(union) = ty {
        let mut non_nil = union.into_vec();
        let had_nil = non_nil.iter().any(|t| matches!(t, LuaType::Nil));
        non_nil.retain(|t| !matches!(t, LuaType::Nil));
        if had_nil
            && let [only] = non_nil.as_slice()
            && is_dotted_class(model, only)
        {
            return format!("({})?", humanize(model, only));
        }
    }
    humanize(model, ty)
}

/// `text` is the rendering of the non-nil `ty`; returns `(text)?` when LuaLS would parenthesise.
pub fn parenthesize_optional_dotted_class(
    model: &SemanticModel<'_>,
    ty: &LuaType,
    text: String,
) -> String {
    if is_dotted_class(model, ty) {
        format!("({text})?")
    } else {
        format!("{text}?")
    }
}

fn is_dotted_class(model: &SemanticModel<'_>, ty: &LuaType) -> bool {
    let (LuaType::Ref(id) | LuaType::Def(id)) = ty else {
        return false;
    };
    id.get_name().contains('.')
        && model
            .type_def_of(id)
            .is_some_and(|def| def.kind == TypeDefKind::Class)
}
