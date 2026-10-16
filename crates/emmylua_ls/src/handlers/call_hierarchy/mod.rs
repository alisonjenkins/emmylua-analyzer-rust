mod build_call_hierarchy;

use build_call_hierarchy::{
    CallHierarchyItemData, build_call_hierarchy_item, build_incoming_hierarchy,
    build_outgoing_hierarchy,
};
use emmylua_code_analysis::{EmmyLuaAnalysis, FileId};
use emmylua_parser::{LuaAstNode, LuaTokenKind};
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    ClientCapabilities, Position, ServerCapabilities,
};
use rowan::TokenAtOffset;
use tokio_util::sync::CancellationToken;

use crate::context::{RequestOutcome, ServerContextSnapshot, snapshot_query};
use crate::handlers::references::reference_searcher::search_references;

use super::RegisterCapabilities;

pub async fn on_prepare_call_hierarchy_handler(
    context: ServerContextSnapshot,
    params: CallHierarchyPrepareParams,
    cancel_token: CancellationToken,
) -> RequestOutcome<Vec<CallHierarchyItem>> {
    let uri = params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;
    snapshot_query(context.analysis(), cancel_token, move |analysis| {
        let file_id = analysis.get_file_id(&uri)?;
        prepare_call_hierarchy(analysis, file_id, position)
    })
    .await
}

pub fn prepare_call_hierarchy(
    analysis: &EmmyLuaAnalysis,
    file_id: FileId,
    position: Position,
) -> Option<Vec<CallHierarchyItem>> {
    let model = analysis.semantic_model(file_id);
    let document = analysis.db.document(file_id)?;
    let root = model.chunk()?;
    let position_offset =
        document.get_offset(position.line as usize, position.character as usize)?;

    if position_offset > root.syntax().text_range().end() {
        return None;
    }

    let token = match root.syntax().token_at_offset(position_offset) {
        TokenAtOffset::Single(token) => token,
        TokenAtOffset::Between(left, right) => {
            if left.kind() == LuaTokenKind::TkName.into() {
                left
            } else {
                right
            }
        }
        TokenAtOffset::None => {
            return None;
        }
    };

    let semantic_decl = model.find_decl(token.into())?;

    Some(vec![build_call_hierarchy_item(
        &model,
        &analysis.db,
        &semantic_decl,
    )?])
}

pub async fn on_incoming_calls_handler(
    context: ServerContextSnapshot,
    params: CallHierarchyIncomingCallsParams,
    cancel_token: CancellationToken,
) -> RequestOutcome<Vec<CallHierarchyIncomingCall>> {
    let item = params.item;
    snapshot_query(context.analysis(), cancel_token, move |analysis| {
        incoming_calls(analysis, &item)
    })
    .await
}

pub fn incoming_calls(
    analysis: &EmmyLuaAnalysis,
    item: &CallHierarchyItem,
) -> Option<Vec<CallHierarchyIncomingCall>> {
    let data = item.data.as_ref()?;
    let data = serde_json::from_value::<CallHierarchyItemData>(data.clone()).ok()?;
    data.semantic_decl.to_semantic_id()?;

    // The references of the function name, so calls found through aliases and typed receivers count.
    let file_id = analysis.get_file_id(&item.uri)?;
    let model = analysis.semantic_model(file_id);
    let document = analysis.db.document(file_id)?;
    let start = item.selection_range.start;
    let offset = document.get_offset(start.line as usize, start.character as usize)?;
    let token = model
        .chunk()?
        .syntax()
        .token_at_offset(offset)
        .right_biased()?;
    let references = search_references(&model, &analysis.db, token, false)?;
    let ranges = references
        .iter()
        .filter_map(|location| {
            let file_id = analysis.get_file_id(&location.uri)?;
            let document = analysis.db.document(file_id)?;
            let range = document.to_rowan_range(location.range)?;
            Some((file_id, range))
        })
        .collect();
    build_incoming_hierarchy(&analysis.db, ranges)
}

pub async fn on_outgoing_calls_handler(
    context: ServerContextSnapshot,
    params: CallHierarchyOutgoingCallsParams,
    cancel_token: CancellationToken,
) -> RequestOutcome<Vec<CallHierarchyOutgoingCall>> {
    let item = params.item;
    snapshot_query(context.analysis(), cancel_token, move |analysis| {
        outgoing_calls(analysis, &item)
    })
    .await
}

pub fn outgoing_calls(
    analysis: &EmmyLuaAnalysis,
    item: &CallHierarchyItem,
) -> Option<Vec<CallHierarchyOutgoingCall>> {
    let file_id = analysis.get_file_id(&item.uri)?;
    build_outgoing_hierarchy(&analysis.db, file_id, item.selection_range.start)
}

pub struct CallHierarchyCapabilities;

impl RegisterCapabilities for CallHierarchyCapabilities {
    fn register_capabilities(server_capabilities: &mut ServerCapabilities, _: &ClientCapabilities) {
        server_capabilities.call_hierarchy_provider = Some(true.into());
    }
}
