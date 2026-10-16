use std::collections::{HashMap, HashSet};

use emmylua_code_analysis::{EmmyLuaAnalysis, FileId, SemanticDatabase, SemanticId, SemanticModel};

use crate::handlers::definition::definition;
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaAstToken, LuaCallExpr, LuaClosureExpr, LuaExpr, LuaFuncStat,
    LuaLocalFuncStat, LuaLocalStat, LuaSyntaxToken, LuaTokenKind, LuaVarExpr, PathTrait,
};
use lsp_types::GotoDefinitionResponse;
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, SymbolKind,
};
use rowan::TextRange;
use serde::{Deserialize, Serialize};

/// Serializable encoding for `SemanticId` (identity = file + range).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SemanticIdData {
    pub kind: String,
    pub file_id: u32,
    pub range: (u32, u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallHierarchyItemData {
    pub semantic_decl: SemanticIdData,
    pub file_id: u32,
}

impl From<&SemanticId> for SemanticIdData {
    fn from(id: &SemanticId) -> Self {
        match id {
            SemanticId::Decl(key) => SemanticIdData {
                kind: "Decl".to_string(),
                file_id: key.file_id.id,
                range: (key.name_range.start().into(), key.name_range.end().into()),
            },
            SemanticId::Member(key) => SemanticIdData {
                kind: "Member".to_string(),
                file_id: key.file_id.id,
                range: (key.key_range.start().into(), key.key_range.end().into()),
            },
            _ => SemanticIdData {
                kind: "Unknown".to_string(),
                file_id: 0,
                range: (0, 0),
            },
        }
    }
}

impl SemanticIdData {
    pub fn to_semantic_id(&self) -> Option<SemanticId> {
        let file_id = FileId::new(self.file_id);
        let range = TextRange::new(self.range.0.into(), self.range.1.into());
        match self.kind.as_str() {
            "Decl" => Some(SemanticId::decl(file_id, range)),
            "Member" => Some(SemanticId::member(file_id, range)),
            _ => None,
        }
    }
}

pub fn build_call_hierarchy_item(
    model: &SemanticModel<'_>,
    db: &SemanticDatabase,
    semantic_decl: &SemanticId,
) -> Option<CallHierarchyItem> {
    let data = CallHierarchyItemData {
        semantic_decl: semantic_decl.into(),
        file_id: model.file_id().id,
    };
    match semantic_decl {
        SemanticId::Decl(key) => {
            let decl_model = model_of(db, key.file_id);
            let decl = decl_model
                .decls()?
                .iter()
                .find(|d| d.id == *semantic_decl)?
                .clone();
            let document = db.document(key.file_id)?;
            let uri = document.get_uri()?;
            let lsp_range = document.to_lsp_range(decl.name_range)?;
            Some(CallHierarchyItem {
                name: decl.name.to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri,
                range: lsp_range,
                selection_range: lsp_range,
                data: Some(serde_json::to_value(data).ok()?),
            })
        }
        SemanticId::Member(key) => {
            let decl_model = model_of(db, key.file_id);
            let member = decl_model
                .members()?
                .iter()
                .find(|m| m.id == *semantic_decl)?
                .clone();
            let document = db.document(key.file_id)?;
            let uri = document.get_uri()?;
            let lsp_range = document.to_lsp_range(key.key_range)?;
            Some(CallHierarchyItem {
                name: member.key.to_path(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri,
                range: lsp_range,
                selection_range: lsp_range,
                data: Some(serde_json::to_value(data).ok()?),
            })
        }
        _ => None,
    }
}

/// Incoming calls from the reference sites of a function; sites that are not calls are skipped.
pub fn build_incoming_hierarchy(
    db: &SemanticDatabase,
    ranges: Vec<(FileId, TextRange)>,
) -> Option<Vec<CallHierarchyIncomingCall>> {
    let mut callers = CallerGroups::default();
    let mut seen = HashSet::new();
    for (file_id, range) in ranges {
        if !seen.insert((file_id, range)) {
            continue;
        }
        collect_call_site(db, file_id, range, &mut callers);
    }
    Some(callers.into_calls())
}

/// Incoming calls grouped by caller, in first-seen order.
#[derive(Default)]
struct CallerGroups {
    index: HashMap<(FileId, Option<TextRange>), usize>,
    calls: Vec<CallHierarchyIncomingCall>,
}

impl CallerGroups {
    fn add(
        &mut self,
        key: (FileId, Option<TextRange>),
        from: impl FnOnce() -> Option<CallHierarchyItem>,
        from_range: lsp_types::Range,
    ) {
        if let Some(&slot) = self.index.get(&key) {
            self.calls[slot].from_ranges.push(from_range);
            return;
        }
        let Some(from) = from() else {
            return;
        };
        self.index.insert(key, self.calls.len());
        self.calls.push(CallHierarchyIncomingCall {
            from,
            from_ranges: vec![from_range],
        });
    }

    fn into_calls(mut self) -> Vec<CallHierarchyIncomingCall> {
        for call in &mut self.calls {
            call.from_ranges.sort_by_key(|r| (r.start, r.end));
        }
        self.calls
    }
}

/// A function-like construct that can be the caller of a call expression.
struct CallerFunction {
    name: String,
    name_token: LuaSyntaxToken,
    range: TextRange,
}

fn collect_call_site(
    db: &SemanticDatabase,
    file_id: FileId,
    range: TextRange,
    callers: &mut CallerGroups,
) -> Option<()> {
    let model = SemanticModel::new(db, file_id);
    let document = db.document(file_id)?;
    let chunk = model.chunk()?;
    let token = chunk
        .syntax()
        .token_at_offset(range.start())
        .right_biased()?;
    if token.kind() != LuaTokenKind::TkName.into() || token.text_range().start() != range.start() {
        return None;
    }
    called_expr_of(&token)?;
    let from_range = document.to_lsp_range(token.text_range())?;
    match enclosing_function(&token) {
        Some(caller) => callers.add(
            (file_id, Some(caller.name_token.text_range())),
            || {
                let selection_range = document.to_lsp_range(caller.name_token.text_range())?;
                let data = model.find_decl(caller.name_token.clone().into()).map(|id| {
                    CallHierarchyItemData {
                        semantic_decl: (&id).into(),
                        file_id: file_id.id,
                    }
                });
                Some(CallHierarchyItem {
                    name: caller.name.clone(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    detail: None,
                    uri: document.get_uri()?,
                    range: document.to_lsp_range(caller.range)?,
                    selection_range,
                    data: data.and_then(|data| serde_json::to_value(data).ok()),
                })
            },
            from_range,
        ),
        None => callers.add(
            (file_id, None),
            || {
                let whole_file = document.to_lsp_range(chunk.syntax().text_range())?;
                Some(CallHierarchyItem {
                    name: document
                        .get_file_path()
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    kind: SymbolKind::MODULE,
                    tags: None,
                    detail: None,
                    uri: document.get_uri()?,
                    range: whole_file,
                    selection_range: whole_file,
                    data: None,
                })
            },
            from_range,
        ),
    }
    Some(())
}

/// The call expression whose callee is the name `token` (`f()`, `a.f()`, `a:f()`).
fn called_expr_of(token: &LuaSyntaxToken) -> Option<LuaCallExpr> {
    let callee = LuaExpr::cast(token.parent()?)?;
    if !matches!(callee, LuaExpr::NameExpr(_) | LuaExpr::IndexExpr(_)) {
        return None;
    }
    let call = LuaCallExpr::cast(callee.syntax().parent()?)?;
    (call.get_prefix_expr()?.syntax() == callee.syntax()).then_some(call)
}

/// The nearest named function around `token`: `function f`, `local function f`,
/// or a closure assigned to a name (`M.f = function`, `local f = function`).
/// Anonymous callbacks are looked through to the function that contains them.
fn enclosing_function(token: &LuaSyntaxToken) -> Option<CallerFunction> {
    for node in token.parent_ancestors() {
        if let Some(stat) = LuaFuncStat::cast(node.clone()) {
            let var = stat.get_func_name()?;
            return Some(CallerFunction {
                name: var.get_access_path()?,
                name_token: var_name_token(&var)?,
                range: stat.get_range(),
            });
        }
        if let Some(stat) = LuaLocalFuncStat::cast(node.clone()) {
            let name = stat.get_local_name()?.get_name_token()?;
            return Some(CallerFunction {
                name: name.get_name_text().to_string(),
                name_token: name.syntax().clone(),
                range: stat.get_range(),
            });
        }
        if let Some(closure) = LuaClosureExpr::cast(node)
            && let Some(caller) = closure_assigned_name(&closure)
        {
            return Some(caller);
        }
    }
    None
}

fn closure_assigned_name(closure: &LuaClosureExpr) -> Option<CallerFunction> {
    let parent = closure.syntax().parent()?;
    if let Some(assign) = LuaAssignStat::cast(parent.clone()) {
        let (vars, exprs) = assign.get_var_and_expr_list();
        let slot = exprs
            .iter()
            .position(|expr| expr.syntax() == closure.syntax())?;
        let var = vars.get(slot)?;
        return Some(CallerFunction {
            name: var.get_access_path()?,
            name_token: var_name_token(var)?,
            range: assign.get_range(),
        });
    }
    let local = LuaLocalStat::cast(parent)?;
    let name = local
        .get_local_name_by_value(closure.clone().into())?
        .get_name_token()?;
    Some(CallerFunction {
        name: name.get_name_text().to_string(),
        name_token: name.syntax().clone(),
        range: local.get_range(),
    })
}

fn var_name_token(var: &LuaVarExpr) -> Option<LuaSyntaxToken> {
    match var {
        LuaVarExpr::NameExpr(name) => name.get_name_token().map(|t| t.syntax().clone()),
        LuaVarExpr::IndexExpr(index) => index.get_name_token().map(|t| t.syntax().clone()),
    }
}

/// Calls made by the function whose name sits at `position`, grouped per callee.
pub fn build_outgoing_hierarchy(
    analysis: &EmmyLuaAnalysis,
    file_id: FileId,
    position: lsp_types::Position,
) -> Option<Vec<CallHierarchyOutgoingCall>> {
    let db = &analysis.db;
    let model = SemanticModel::new(db, file_id);
    let document = db.document(file_id)?;
    let chunk = model.chunk()?;
    let offset = document.get_offset(position.line as usize, position.character as usize)?;
    let name_token = chunk.syntax().token_at_offset(offset).right_biased()?;
    let body = function_body_named_by(&name_token)?;

    let mut slots: HashMap<SemanticIdData, usize> = HashMap::new();
    let mut calls: Vec<CallHierarchyOutgoingCall> = Vec::new();
    for call in body.descendants::<LuaCallExpr>() {
        let Some(callee) = callee_name_token(&call) else {
            continue;
        };
        let Some(from_range) = document.to_lsp_range(callee.text_range()) else {
            continue;
        };
        let mut targets: Vec<SemanticId> =
            model.find_decl(callee.clone().into()).into_iter().collect();
        for origin in definition_targets(analysis, file_id, from_range.start) {
            if !targets.contains(&origin) {
                targets.push(origin);
            }
        }
        if targets.is_empty() {
            targets = self_method_targets(&model, &call, &callee);
        }
        for target in targets {
            let key = SemanticIdData::from(&target);
            if let Some(&slot) = slots.get(&key) {
                calls[slot].from_ranges.push(from_range);
                continue;
            }
            let Some(to) = build_call_hierarchy_item(&model, db, &target) else {
                continue;
            };
            slots.insert(key, calls.len());
            calls.push(CallHierarchyOutgoingCall {
                to,
                from_ranges: vec![from_range],
            });
        }
    }
    Some(calls)
}

/// The members `self:name()` / `self.name()` can reach when go-to-definition finds nothing: the
/// fields named `name` of the table the enclosing method is defined on. The type of `self` is not
/// needed, so classes built by a factory call or annotated more than once still resolve.
fn self_method_targets(
    model: &SemanticModel<'_>,
    call: &LuaCallExpr,
    callee: &LuaSyntaxToken,
) -> Vec<SemanticId> {
    let Some(LuaExpr::IndexExpr(index)) = call.get_prefix_expr() else {
        return Vec::new();
    };
    let Some(LuaExpr::NameExpr(receiver)) = index.get_prefix_expr() else {
        return Vec::new();
    };
    if receiver.get_name_text().as_deref() != Some("self") {
        return Vec::new();
    }
    let Some(receiver_token) = receiver.get_name_token() else {
        return Vec::new();
    };
    let Some(self_id) = model.find_decl(receiver_token.syntax().clone().into()) else {
        return Vec::new();
    };
    let Some(owner) = model
        .decls()
        .and_then(|decls| decls.iter().find(|decl| decl.id == self_id))
        .and_then(|decl| decl.self_owner.clone())
    else {
        return Vec::new();
    };
    model
        .members_of_owner_named(&owner, callee.text())
        .as_slice()
        .iter()
        .map(|member| member.id.clone())
        .collect()
}

/// Where go-to-definition at the call's callee lands, as declarations: LuaLS lists each of these
/// as a definition of the call (the field and the function it is set to, every file assigning a
/// global).
fn definition_targets(
    analysis: &EmmyLuaAnalysis,
    file_id: FileId,
    position: lsp_types::Position,
) -> Vec<SemanticId> {
    let locations = match definition(analysis, file_id, position) {
        Some(GotoDefinitionResponse::Scalar(location)) => vec![location],
        Some(GotoDefinitionResponse::Array(locations)) => locations,
        _ => Vec::new(),
    };
    locations
        .iter()
        .filter_map(|location| {
            let target_file = analysis.get_file_id(&location.uri)?;
            let document = analysis.db.document(target_file)?;
            let offset = document.to_rowan_range(location.range)?.start();
            let target_model = SemanticModel::new(&analysis.db, target_file);
            let token = target_model
                .chunk()?
                .syntax()
                .token_at_offset(offset)
                .right_biased()?;
            if token.kind() != LuaTokenKind::TkName.into() {
                return None;
            }
            target_model.find_decl(token.into())
        })
        .collect()
}

/// The closure of the function named by `name_token` (the `f` of `function f`,
/// `local function f`, `M.f = function` or `local f = function`).
fn function_body_named_by(name_token: &LuaSyntaxToken) -> Option<LuaClosureExpr> {
    for node in name_token.parent_ancestors() {
        if let Some(stat) = LuaFuncStat::cast(node.clone()) {
            return stat.get_closure();
        }
        if let Some(stat) = LuaLocalFuncStat::cast(node.clone()) {
            return stat.get_closure();
        }
        if let Some(assign) = LuaAssignStat::cast(node.clone()) {
            let (vars, exprs) = assign.get_var_and_expr_list();
            let slot = vars.iter().position(|var| {
                var_name_token(var).is_some_and(|t| t.text_range() == name_token.text_range())
            })?;
            return match exprs.get(slot)? {
                LuaExpr::ClosureExpr(closure) => Some(closure.clone()),
                _ => None,
            };
        }
        if let Some(local) = LuaLocalStat::cast(node) {
            let names: Vec<_> = local.get_local_name_list().collect();
            let slot = names.iter().position(|name| {
                name.get_name_token()
                    .is_some_and(|t| t.syntax().text_range() == name_token.text_range())
            })?;
            return match local.get_value_exprs().nth(slot)? {
                LuaExpr::ClosureExpr(closure) => Some(closure),
                _ => None,
            };
        }
    }
    None
}

/// The name token a call is made through (`f()` -> `f`, `a.b:f()` -> `f`).
fn callee_name_token(call: &LuaCallExpr) -> Option<LuaSyntaxToken> {
    match call.get_prefix_expr()? {
        LuaExpr::NameExpr(name) => name.get_name_token().map(|t| t.syntax().clone()),
        LuaExpr::IndexExpr(index) => index.get_name_token().map(|t| t.syntax().clone()),
        _ => None,
    }
}

fn model_of<'a>(db: &'a SemanticDatabase, file_id: FileId) -> SemanticModel<'a> {
    SemanticModel::new(db, file_id)
}
