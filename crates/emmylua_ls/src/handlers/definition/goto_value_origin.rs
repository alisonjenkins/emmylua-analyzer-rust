//! LuaLS lists, beside a field or variable, where its assigned value comes from: the function a
//! field is set to (`M.f = local_fn`) and the closure a called function returns (`x = make()`).

use emmylua_code_analysis::{EmmyLuaAnalysis, SemanticId, SemanticModel};
use lsp_types::GotoDefinitionResponse;

use super::goto_def_definition::location_of;
use crate::handlers::common::value_origin_sites;

pub(super) fn with_value_origin(
    analysis: &EmmyLuaAnalysis,
    model: &SemanticModel<'_>,
    decl: &SemanticId,
    response: GotoDefinitionResponse,
) -> GotoDefinitionResponse {
    let mut locations = match response {
        GotoDefinitionResponse::Scalar(location) => vec![location],
        GotoDefinitionResponse::Array(locations) => locations,
        link @ GotoDefinitionResponse::Link(_) => return link,
    };
    for (file_id, range) in value_origin_sites(model, decl) {
        if let Some(location) = location_of(&analysis.db, file_id, range)
            && !locations.contains(&location)
        {
            locations.push(location);
        }
    }
    match locations.len() {
        1 => GotoDefinitionResponse::Scalar(locations.remove(0)),
        _ => GotoDefinitionResponse::Array(locations),
    }
}
