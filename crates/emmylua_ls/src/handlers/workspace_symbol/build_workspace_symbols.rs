use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashSet},
    sync::{Arc, Mutex},
};

use emmylua_code_analysis::{
    Decl, DeclKind, EmmyLuaAnalysis, FileFacts, LuaDocument, Member, SemanticId, SemanticModel,
};
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaExpr, LuaIndexExpr, LuaSyntaxKind, LuaSyntaxNode,
};
use lsp_types::{SymbolKind, SymbolTag, Uri, WorkspaceSymbol, WorkspaceSymbolResponse};
use rowan::TextRange;
use tokio_util::sync::CancellationToken;

use super::rank::{MatchTier, match_tier, path_rank};

/// LuaLS stops at 1000 results; a one-letter query otherwise returns tens of thousands of symbols.
const MAX_WORKSPACE_SYMBOLS: usize = 1000;

/// Sort key: best match first, then project code before vendored code (library,
/// std or remote files, and files under a dot-prefixed directory), then shorter
/// names, shallower paths and a fixed location order so the same workspace
/// always yields the same list.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RankKey {
    tier: MatchTier,
    vendored: bool,
    name_len: usize,
    depth: usize,
    uri: Arc<str>,
    line: u32,
    column: u32,
}

struct Candidate {
    key: RankKey,
    symbol: WorkspaceSymbol,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

/// Collects the best `MAX_WORKSPACE_SYMBOLS` matches of one file (a max-heap, worst on top).
struct FileSymbols<'a> {
    document: LuaDocument<'a>,
    uri: Uri,
    uri_key: Arc<str>,
    vendored: bool,
    depth: usize,
    query: &'a str,
    best: BinaryHeap<Candidate>,
}

impl FileSymbols<'_> {
    fn add(
        &mut self,
        name: &str,
        range: TextRange,
        build: impl FnOnce() -> (SymbolKind, Option<Vec<SymbolTag>>, Option<String>),
    ) {
        let Some(tier) = match_tier(name, self.query) else {
            return;
        };
        let Some(lsp_range) = self.document.to_lsp_range(range) else {
            return;
        };
        let key = RankKey {
            tier,
            vendored: self.vendored,
            name_len: name.len(),
            depth: self.depth,
            uri: self.uri_key.clone(),
            line: lsp_range.start.line,
            column: lsp_range.start.character,
        };
        if self.best.len() >= MAX_WORKSPACE_SYMBOLS
            && self.best.peek().is_some_and(|worst| key >= worst.key)
        {
            return;
        }
        let (kind, tags, container_name) = build();
        self.best.push(Candidate {
            key,
            symbol: WorkspaceSymbol {
                name: name.to_string(),
                kind,
                tags,
                location: lsp_types::OneOf::Left(lsp_types::Location {
                    uri: self.uri.clone(),
                    range: lsp_range,
                }),
                container_name,
                data: None,
            },
        });
        if self.best.len() > MAX_WORKSPACE_SYMBOLS {
            self.best.pop();
        }
    }
}

/// Keeps the best `MAX_WORKSPACE_SYMBOLS` of `candidates`, in no particular order.
fn trim_to_best(candidates: &mut Vec<Candidate>) {
    if candidates.len() > MAX_WORKSPACE_SYMBOLS {
        candidates.select_nth_unstable(MAX_WORKSPACE_SYMBOLS);
        candidates.truncate(MAX_WORKSPACE_SYMBOLS);
    }
}

pub fn build_workspace_symbols(
    analysis: &EmmyLuaAnalysis,
    query: String,
    cancel_token: CancellationToken,
) -> Option<WorkspaceSymbolResponse> {
    let merged: Mutex<Vec<Candidate>> = Mutex::new(Vec::new());
    let roots = analysis.db.workspace_root_paths();
    analysis.db.parallel_for_each_file(|file_id, model| {
        if cancel_token.is_cancelled() {
            return;
        }
        let Some(document) = model.document(file_id) else {
            return;
        };
        let Some(uri) = document.get_uri() else {
            return;
        };
        let location = analysis
            .db
            .file_path(file_id)
            .map(|path| path_rank(&path, &roots));
        let outside_main = analysis
            .db
            .workspace_id_of(file_id)
            .is_some_and(|id| !id.is_main());
        let mut file = FileSymbols {
            document,
            uri_key: Arc::from(uri.as_str()),
            vendored: outside_main || location.is_some_and(|rank| rank.hidden_dir),
            depth: location.map_or(usize::MAX, |rank| rank.depth),
            uri,
            query: &query,
            best: BinaryHeap::new(),
        };
        if let Some(decls) = model.decls() {
            for decl in decls.iter() {
                if cancel_token.is_cancelled() {
                    return;
                }
                let kind = match decl.kind {
                    DeclKind::Global => SymbolKind::FUNCTION,
                    DeclKind::Local { .. } | DeclKind::Param => {
                        if decl.is_implicit_self || decl.name == "..." {
                            continue;
                        }
                        local_symbol_kind(decl)
                    }
                };
                file.add(&decl.name, decl.name_range, || (kind, None, None));
            }
        }
        push_reassignments(model, &mut file, &cancel_token);
        if let Some(facts) = model.file_facts() {
            for def in facts.type_defs.iter() {
                if cancel_token.is_cancelled() {
                    return;
                }
                file.add(&def.full_name, def.name_range, || {
                    (SymbolKind::CLASS, None, None)
                });
            }
            // `local t = { a = 1 }` is collected twice (under the local and under the
            // anonymous table) with the same id; keep the first, which names the local.
            let mut seen_members = HashSet::new();
            for member in facts.members.iter() {
                if cancel_token.is_cancelled() {
                    return;
                }
                if !seen_members.insert(&member.id) {
                    continue;
                }
                let Some(name) = member.key.get_name() else {
                    continue;
                };
                let Some(key_range) = member.id.member_key_range() else {
                    continue;
                };
                file.add(name, key_range, || {
                    (
                        member_symbol_kind(member),
                        member.deprecated.then(|| vec![SymbolTag::DEPRECATED]),
                        member_container_name(facts, member),
                    )
                });
                push_global_path(model, facts, member, name, key_range, &mut file);
            }
        }
        let mut merged = merged.lock().expect("workspace symbols lock");
        merged.extend(file.best);
        if merged.len() > 2 * MAX_WORKSPACE_SYMBOLS {
            trim_to_best(&mut merged);
        }
    });
    if cancel_token.is_cancelled() {
        return Some(WorkspaceSymbolResponse::Nested(Vec::new()));
    }
    let mut best = merged.into_inner().expect("workspace symbols lock");
    trim_to_best(&mut best);
    best.sort();
    Some(WorkspaceSymbolResponse::Nested(
        best.into_iter().map(|candidate| candidate.symbol).collect(),
    ))
}

/// LuaLS lists every plain assignment to a name (`plan = patch()`) next to its
/// declaration, whatever the name resolves to. Only name uses whose text matches
/// the query have their syntax node looked up.
fn push_reassignments(
    model: &SemanticModel<'_>,
    file: &mut FileSymbols<'_>,
    cancel_token: &CancellationToken,
) {
    let (Some(uses), Some(tree)) = (model.name_uses(), model.syntax_tree()) else {
        return;
    };
    let mut root = None;
    let mut decl_starts = None;
    for name_use in uses {
        if cancel_token.is_cancelled() {
            return;
        }
        if match_tier(&name_use.name, file.query).is_none() {
            continue;
        }
        let root = root.get_or_insert_with(|| tree.get_red_root());
        let Some(node) = name_use.syntax.to_node_from_root(root) else {
            continue;
        };
        if !is_assignment_target(&node) {
            continue;
        }
        // The first assignment to a global is already its declaration.
        let decl_starts = decl_starts.get_or_insert_with(|| {
            model
                .decls()
                .unwrap_or_default()
                .iter()
                .map(|decl| decl.name_range.start())
                .collect::<HashSet<_>>()
        });
        let range = name_use.syntax.get_range();
        if decl_starts.contains(&range.start()) {
            continue;
        }
        file.add(&name_use.name, range, || (SymbolKind::VARIABLE, None, None));
    }
}

fn is_assignment_target(node: &LuaSyntaxNode) -> bool {
    node.parent()
        .and_then(LuaAssignStat::cast)
        .is_some_and(|assign| {
            let (vars, _) = assign.get_var_and_expr_list();
            vars.iter().any(|var| var.syntax() == node)
        })
}

/// A path longer than this is not a real `a.b.c = v` chain; it also bounds a cyclic owner chain.
const MAX_PATH_SEGMENTS: usize = 32;

/// LuaLS also lists an assignment to a field of a global as the whole dotted
/// path (`vim.ui.input = ...` is `vim.ui.input`, kind Variable) next to the
/// field symbol. Table constructor fields and paths rooted in a local get no
/// such symbol. The syntax tree is only consulted for paths that match.
fn push_global_path(
    model: &SemanticModel<'_>,
    facts: &FileFacts,
    member: &Member,
    name: &str,
    key_range: TextRange,
    file: &mut FileSymbols<'_>,
) {
    let known = match global_path(facts, member, name) {
        Some(GlobalPath::Known(path)) => Some(path),
        Some(GlobalPath::FromSyntax) => None,
        None => return,
    };
    if known
        .as_ref()
        .is_some_and(|path| match_tier(path, file.query).is_none())
    {
        return;
    }
    let Some(index_expr) = assigned_index_expr(model, key_range) else {
        return;
    };
    let path = match known {
        Some(path) => path,
        None => {
            let Some(path) = syntax_path(&LuaExpr::IndexExpr(index_expr)) else {
                return;
            };
            if match_tier(&path, file.query).is_none() {
                return;
            }
            path
        }
    };
    file.add(&path, key_range, || (SymbolKind::VARIABLE, None, None));
}

enum GlobalPath {
    Known(String),
    /// The analysis names the owner of a path with three or more segments
    /// wrongly (`a.b.b.c`), so the path has to be read from the source.
    FromSyntax,
}

fn global_path(facts: &FileFacts, member: &Member, name: &str) -> Option<GlobalPath> {
    let mut segments = vec![name];
    let mut owner = &member.owner;
    loop {
        if segments.len() > MAX_PATH_SEGMENTS {
            return None;
        }
        match owner {
            SemanticId::Name(root) => {
                if root.matches('.').count() >= 2 {
                    return Some(GlobalPath::FromSyntax);
                }
                segments.push(root);
                break;
            }
            SemanticId::Decl(_) => {
                let decl = facts.decl_by_id(owner)?;
                if !matches!(decl.kind, DeclKind::Global) {
                    return None;
                }
                segments.push(&decl.name);
                break;
            }
            SemanticId::Member(_) => {
                let parent = facts.member_by_id(owner)?;
                segments.push(parent.key.get_name()?);
                owner = &parent.owner;
            }
            SemanticId::TypeDef(_) | SemanticId::Signature(_) => return None,
        }
    }
    segments.reverse();
    Some(GlobalPath::Known(segments.join(".")))
}

/// The index expression whose field is the member key at `range` (`a.b = v`,
/// `function a.b() end`); a table constructor key has none.
fn assigned_index_expr(model: &SemanticModel<'_>, range: TextRange) -> Option<LuaIndexExpr> {
    let token = model
        .syntax_tree()?
        .get_red_root()
        .token_at_offset(range.start())
        .right_biased()?;
    token
        .parent_ancestors()
        .take(2)
        .find_map(LuaIndexExpr::cast)
}

/// `a.b.c` as written; `None` for anything that is not a plain name chain.
fn syntax_path(expr: &LuaExpr) -> Option<String> {
    match expr {
        LuaExpr::NameExpr(name) => name.get_name_text(),
        LuaExpr::IndexExpr(index) => {
            let prefix = syntax_path(&index.get_prefix_expr()?)?;
            Some(format!(
                "{prefix}.{}",
                index.get_index_key()?.get_path_part()
            ))
        }
        LuaExpr::ParenExpr(paren) => syntax_path(&paren.get_expr()?),
        _ => None,
    }
}

fn member_symbol_kind(member: &Member) -> SymbolKind {
    if member.is_method {
        return SymbolKind::METHOD;
    }
    let is_function = member.value_syntax.is_some_and(|syntax| {
        matches!(
            syntax.get_kind(),
            LuaSyntaxKind::ClosureExpr | LuaSyntaxKind::TypeFun
        )
    });
    if is_function {
        SymbolKind::FUNCTION
    } else {
        SymbolKind::FIELD
    }
}

fn member_container_name(facts: &FileFacts, member: &Member) -> Option<String> {
    match &member.owner {
        SemanticId::TypeDef(key) => Some(key.full_name.to_string()),
        SemanticId::Name(name) => Some(name.to_string()),
        SemanticId::Decl(_) => facts
            .decl_by_id(&member.owner)
            .map(|decl| decl.name.to_string()),
        SemanticId::Member(_) => facts
            .member_by_id(&member.owner)
            .and_then(|owner| owner.key.get_name())
            .map(str::to_string),
        SemanticId::Signature(_) => None,
    }
}

// LuaLS reports every local as Variable; functions are told apart here so
// clients can filter on them.
fn local_symbol_kind(decl: &Decl) -> SymbolKind {
    let is_function = decl
        .value_expr_syntax
        .is_some_and(|syntax| syntax.get_kind() == LuaSyntaxKind::ClosureExpr);
    if is_function {
        SymbolKind::FUNCTION
    } else {
        SymbolKind::VARIABLE
    }
}
