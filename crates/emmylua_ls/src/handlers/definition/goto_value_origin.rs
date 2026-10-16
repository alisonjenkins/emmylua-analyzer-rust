//! LuaLS lists, beside a field or variable, where its assigned value comes from: the function a
//! field is set to (`M.f = local_fn`) and the closure a called function returns (`x = make()`).

use emmylua_code_analysis::{EmmyLuaAnalysis, SemanticId, SemanticModel};
use emmylua_parser::LuaSyntaxToken;
use lsp_types::{GotoDefinitionResponse, Location};

use super::goto_def_definition::{goto_def_definition, location_of};
use crate::handlers::common::{call_value_function, is_function_valued, resolve_alias_origin};

pub(super) fn with_value_origin(
    analysis: &EmmyLuaAnalysis,
    model: &SemanticModel<'_>,
    decl: &SemanticId,
    token: &LuaSyntaxToken,
    response: GotoDefinitionResponse,
) -> GotoDefinitionResponse {
    let mut locations = match response {
        GotoDefinitionResponse::Scalar(location) => vec![location],
        GotoDefinitionResponse::Array(locations) => locations,
        link @ GotoDefinitionResponse::Link(_) => return link,
    };
    let mut extra: Vec<Location> = Vec::new();
    if matches!(decl, SemanticId::Member(_))
        && let Some(origin) = resolve_alias_origin(model, decl)
        && origin != *decl
        && is_function_valued(model, &origin)
    {
        match &origin {
            SemanticId::Member(key) => {
                extra.extend(location_of(&analysis.db, key.file_id, key.key_range))
            }
            _ => match goto_def_definition(model, &analysis.db, &origin, token) {
                Some(GotoDefinitionResponse::Scalar(location)) => extra.push(location),
                Some(GotoDefinitionResponse::Array(found)) => extra.extend(found),
                _ => {}
            },
        }
    }
    if let Some((file_id, range)) = call_value_function(model, decl)
        && let Some(location) = location_of(&analysis.db, file_id, range)
    {
        extra.push(location);
    }
    for location in extra {
        if !locations.contains(&location) {
            locations.push(location);
        }
    }
    match locations.len() {
        1 => GotoDefinitionResponse::Scalar(locations.remove(0)),
        _ => GotoDefinitionResponse::Array(locations),
    }
}
