use emmylua_code_analysis::{LuaType, SemanticModel, humanize_semantic_type_hover};

pub fn humanize(model: &SemanticModel<'_>, ty: &LuaType) -> String {
    humanize_semantic_type_hover(model, ty)
}
