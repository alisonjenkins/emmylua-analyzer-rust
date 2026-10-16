//! LuaLS answers go-to-definition on a field with the field's declaration plus every assignment
//! of it made through the same receiver in the same file (`fs.clone = a`, `fs.clone = b`).

use emmylua_code_analysis::{EmmyLuaAnalysis, SemanticId, SemanticModel};
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaExpr, LuaFuncStat, LuaIndexExpr, LuaNameExpr, LuaSyntaxToken,
};
use lsp_types::{GotoDefinitionResponse, Location};

use crate::handlers::references::reference_searcher::search_references;

pub(super) fn with_field_assignments(
    analysis: &EmmyLuaAnalysis,
    model: &SemanticModel<'_>,
    token: &LuaSyntaxToken,
    response: GotoDefinitionResponse,
) -> GotoDefinitionResponse {
    let Some(cursor) = token
        .parent()
        .and_then(LuaIndexExpr::cast)
        .and_then(|index_expr| receiver_of(model, &index_expr))
    else {
        return response;
    };
    let mut locations = match response {
        GotoDefinitionResponse::Scalar(location) => vec![location],
        GotoDefinitionResponse::Array(locations) => locations,
        link @ GotoDefinitionResponse::Link(_) => return link,
    };
    let references =
        search_references(model, &analysis.db, token.clone(), true).unwrap_or_default();
    for location in references {
        if !locations.contains(&location)
            && is_assignment_through(analysis, model, &location, &cursor)
        {
            locations.push(location);
        }
    }
    match locations.len() {
        1 => GotoDefinitionResponse::Scalar(locations.remove(0)),
        _ => GotoDefinitionResponse::Array(locations),
    }
}

/// The variable a field is reached through: `a.b` in `a.b.c`.
#[derive(PartialEq)]
struct Receiver {
    text: String,
    root_name: String,
    /// `None` for a global and for `self`, which is a different declaration in every method.
    root_decl: Option<SemanticId>,
}

fn receiver_of(model: &SemanticModel<'_>, index_expr: &LuaIndexExpr) -> Option<Receiver> {
    let prefix = index_expr.get_prefix_expr()?;
    let mut root = prefix.clone();
    while let LuaExpr::IndexExpr(inner) = root {
        root = inner.get_prefix_expr()?;
    }
    let LuaExpr::NameExpr(root_name) = root else {
        return None;
    };
    let name = root_name.get_name_text()?;
    let root_decl = (name != "self")
        .then(|| root_decl_of(model, &root_name))
        .flatten();
    Some(Receiver {
        text: prefix.syntax().text().to_string(),
        root_name: name,
        root_decl,
    })
}

fn root_decl_of(model: &SemanticModel<'_>, root: &LuaNameExpr) -> Option<SemanticId> {
    match model.resolve_name(root.get_position())? {
        decl @ SemanticId::Decl(_) => Some(decl),
        _ => None,
    }
}

fn is_assignment_through(
    analysis: &EmmyLuaAnalysis,
    model: &SemanticModel<'_>,
    location: &Location,
    cursor: &Receiver,
) -> bool {
    if analysis.get_file_id(&location.uri) != Some(model.file_id()) {
        return false;
    }
    let Some(document) = analysis.db.document(model.file_id()) else {
        return false;
    };
    let Some(chunk) = model.chunk() else {
        return false;
    };
    let Some(offset) = document.get_offset(
        location.range.start.line as usize,
        location.range.start.character as usize,
    ) else {
        return false;
    };
    let Some(index_expr) = chunk
        .syntax()
        .token_at_offset(offset)
        .right_biased()
        .and_then(|token| token.parent())
        .and_then(LuaIndexExpr::cast)
    else {
        return false;
    };
    let Some(owner) = index_expr.syntax().parent() else {
        return false;
    };
    let is_target = LuaFuncStat::can_cast(owner.kind().into())
        || LuaAssignStat::cast(owner).is_some_and(|assign| {
            assign
                .get_var_and_expr_list()
                .0
                .iter()
                .any(|var| var.get_range() == index_expr.get_range())
        });
    // `Context.f = v` and `self.f = v` in a method of `Context` set the same field.
    is_target
        && receiver_of(model, &index_expr).is_some_and(|site| {
            site == *cursor || site.root_name == "self" || cursor.root_name == "self"
        })
}
