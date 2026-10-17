use super::member::member_version_visible;
use super::prelude::*;
use crate::check::checker::generic_constraint_mismatch::resolved_call_signatures;
use crate::check::checker::param_count::callable_functions;
use crate::check::checker::param_type_check::callable_candidates_uncached;
use emmylua_parser::LuaAstToken;

/// Owner placeholder for a member access whose receiver is not a name chain (`open():close()`).
const EXPR_OWNER: &str = "<expr>";

impl<'db> SemanticModel<'db> {
    /// Currently configured runtime version (used for `---@version` visibility checks).
    pub fn lua_version(&self) -> Option<LuaVersionNumber> {
        self.db().lua_version()
    }
    /// Whether the member's `---@version` conditions admit the configured runtime version.
    pub fn is_member_ref_version_visible(&self, member_ref: &MemberRef) -> bool {
        let Some(facts) = self.file_facts_of(member_ref.file_id) else {
            return true;
        };
        let Some(member) = facts.member_by_id(&member_ref.id) else {
            return true;
        };
        member_version_visible(self, facts, member)
    }
    /// For a member whose `---@version` excludes the configured runtime, the same-file, same-key sibling of its
    /// owner that the runtime admits (`coroutine.running` is declared once per Lua version). `None` when the
    /// member is itself visible or has no visible sibling.
    pub fn version_visible_sibling(&self, member: &SemanticId) -> Option<SemanticId> {
        let SemanticId::Member(key) = member else {
            return None;
        };
        let current = self.file_facts_of(key.file_id)?.member_by_id(member)?;
        let siblings = self.members_of_owner_named(&current.owner, current.key.to_path().as_str());
        let current_ref = siblings.iter().find(|r| &r.id == member)?;
        if self.is_member_ref_version_visible(current_ref) {
            return None;
        }
        siblings
            .iter()
            .find(|r| r.file_id == key.file_id && self.is_member_ref_version_visible(r))
            .map(|r| r.id.clone())
    }
    /// A model for any file on the same analysis database.
    pub fn model_for(&self, file_id: FileId) -> Self {
        SemanticModel::with_closure_returns(self.db(), file_id, self.closure_returns.clone())
    }
    pub fn file_id(&self) -> FileId {
        self.view.file_id()
    }
    pub fn file_path(&self) -> Option<std::path::PathBuf> {
        self.db().file_path(self.view.file_id())
    }
    pub fn document(&self, file_id: FileId) -> Option<LuaDocument<'db>> {
        self.db().document(file_id)
    }
    pub fn document_current(&self) -> Option<LuaDocument<'db>> {
        self.document(self.view.file_id())
    }
    pub fn strict_array_index(&self) -> bool {
        self.db().strict_array_index()
    }
    pub fn file_ids(&self) -> Vec<FileId> {
        self.db().file_ids()
    }
    pub fn main_workspace_file_ids(&self) -> Vec<FileId> {
        self.db().main_workspace_file_ids()
    }
    pub fn file_path_of(&self, file_id: FileId) -> Option<std::path::PathBuf> {
        self.db().file_path(file_id)
    }
    pub fn file_uri_of(&self, file_id: FileId) -> Option<lsp_types::Uri> {
        self.db().file_uri(file_id)
    }
    pub fn module_name_of(&self, file_id: FileId) -> Option<String> {
        self.db().module_name_of(file_id)
    }
    pub fn module_names_of(&self, file_id: FileId) -> Vec<String> {
        self.db().module_names_of(file_id)
    }
    pub fn module_info_of(&self, file_id: FileId) -> Option<crate::ModuleInfo> {
        self.db().module_info_of(file_id)
    }
    pub fn workspace_id_of(&self, file_id: FileId) -> Option<WorkspaceId> {
        self.db().workspace_id_of(file_id)
    }
    pub fn is_std_file(&self, file_id: FileId) -> bool {
        self.db().is_std_file(file_id)
    }
    pub fn is_main_file(&self, file_id: FileId) -> bool {
        self.db().is_main_file(file_id)
    }
    // -- File / syntax --

    pub fn syntax_tree(&self) -> Option<&'db LuaSyntaxTree> {
        self.view.syntax_tree()
    }
    /// Syntax tree of any file (used to locate cross-file doc type nodes).
    pub fn syntax_tree_of(&self, file_id: FileId) -> Option<&'db LuaSyntaxTree> {
        self.analysis().syntax_tree(file_id)
    }
    pub fn chunk(&self) -> Option<LuaChunk> {
        self.view.chunk()
    }
    /// Innermost closure containing `offset`, found by walking token ancestors.
    pub(crate) fn enclosing_closure_at(&self, offset: TextSize) -> Option<LuaClosureExpr> {
        let tree = self.syntax_tree()?;
        let root = tree.get_red_root();
        let token = match root.token_at_offset(offset) {
            rowan::TokenAtOffset::Single(token) => token,
            rowan::TokenAtOffset::Between(_, right) => right,
            rowan::TokenAtOffset::None => return None,
        };
        token.parent_ancestors().find_map(LuaClosureExpr::cast)
    }
    pub fn parse_errors(&self) -> Option<Vec<LuaParseError>> {
        self.view.parse_errors()
    }
    /// Whether the doc tag is in `emmyrc.doc.known_tags` (used by unknown_doc_tag checks).
    pub fn is_known_doc_tag(&self, name: &str) -> bool {
        self.analysis().is_known_doc_tag(self.view.file_id(), name)
    }
    // -- Facts --

    /// Per-file facts arena (decls/scopes/members/... + `---@diagnostic` annotations).
    pub fn file_facts(&self) -> Option<&'db FileFacts> {
        self.view.file_facts()
    }
    /// Cross-file facts (member flags from members_of_owner results, etc.).
    pub fn file_facts_of(&self, file_id: FileId) -> Option<&'db FileFacts> {
        self.analysis().file_facts(file_id)
    }
    /// Exported facts for a file (types/globals/runtime_values/members/module identity layer).
    /// Cross-file consumption entry: computed only on the defining file's facts, never entering the defining file's function bodies.
    pub fn file_exports(&self, file_id: FileId) -> Option<&'db FileExports> {
        self.analysis().file_exports(file_id)
    }
    /// Exported facts for the current file.
    pub fn file_exports_current(&self) -> Option<&'db FileExports> {
        self.file_exports(self.view.file_id())
    }
    pub fn decls(&self) -> Option<&'db [Decl]> {
        self.view.decls()
    }
    pub fn scopes(&self) -> Option<&'db [Scope]> {
        self.view.scopes()
    }
    pub fn members(&self) -> Option<&'db [Member]> {
        self.view.members()
    }
    pub fn signatures(&self) -> Option<&'db [Signature]> {
        self.view.signatures()
    }
    pub fn name_uses(&self) -> Option<&'db [NameUse]> {
        self.view.name_uses()
    }
    pub fn decl_by_offset(&self, offset: TextSize) -> Option<SemanticId> {
        self.view.decl_by_offset(offset)
    }
    // -- Names / references --

    pub fn resolve_name(&self, offset: TextSize) -> Option<SemanticId> {
        let key = (self.view.file_id(), offset);
        if let Some(cached) = self.cache.borrow().resolve_name.get(&key) {
            return cached.clone();
        }
        let result = self.view.resolve_name(offset);
        self.cache
            .borrow_mut()
            .resolve_name
            .insert(key, result.clone());
        result
    }
    /// Resolve a name use to a **local** declaration only.
    ///
    /// Unlike `resolve_name`, this never falls back to the workspace global index.
    /// It is useful for checkers that only need same-file declarations and then
    /// handle cross-file cases with a cheaper precomputed structure.
    pub(crate) fn resolve_local_name(&self, offset: TextSize) -> Option<SemanticId> {
        let facts = self.file_facts()?;
        let name_use = facts.name_use_at_offset(offset)?;
        facts
            .find_visible_decl_before_offset(&name_use.name, offset)
            .map(|decl| decl.id.clone())
    }
    /// Workspace global declaration (cross-file). The `Decl` key carries the declaring file.
    pub fn global_decl(&self, name: &str) -> Option<SemanticId> {
        self.analysis().global_decl(name)
    }
    /// Same-workspace global declarations with this name (overload candidates).
    pub fn global_decls_named_for_file(&self, file_id: FileId, name: &str) -> Vec<SemanticId> {
        self.analysis().global_decls_named_for_file(file_id, name)
    }

    /// First non-empty workspace's global declarations with this name.
    pub fn global_decls_named_primary(&self, name: &str) -> Vec<SemanticId> {
        self.analysis().global_decls_named_primary(name)
    }
    pub fn decl_references(&self, decl: &SemanticId) -> Vec<LuaSyntaxId> {
        self.analysis()
            .decl_references(self.view.file_id(), decl.clone())
    }
    /// Syntax location -> semantic declaration (mirrors old `find_decl`; M0 supports Decl / Member / TypeDef):
    /// Definition name -> declaration name hit -> index member key -> doc name type -> name use point.
    /// `require("mod").field` -> the module file's member for `field`.
    pub fn require_field_member(&self, index_expr: &LuaIndexExpr) -> Option<SemanticId> {
        let LuaExpr::CallExpr(call) = index_expr.get_prefix_expr()? else {
            return None;
        };
        if !call.is_require() {
            return None;
        }
        let arg = call.get_args_list()?.get_args().next()?;
        let LuaLiteralToken::String(module_name) = arg
            .syntax()
            .descendants_with_tokens()
            .filter_map(|it| it.into_token())
            .find_map(LuaLiteralToken::cast)?
        else {
            return None;
        };
        let key = index_expr.get_index_key()?.get_path_part();
        let module_file = self.module_file_of(&module_name.get_value())?;
        self.file_facts_of(module_file)?
            .members
            .iter()
            .find(|member| member.key.to_path() == key)
            .map(|member| member.id.clone())
    }
    pub fn find_decl(
        &self,
        node_or_token: rowan::NodeOrToken<LuaSyntaxNode, LuaSyntaxToken>,
    ) -> Option<SemanticId> {
        let token = match node_or_token {
            rowan::NodeOrToken::Node(node) => node.first_token()?,
            rowan::NodeOrToken::Token(token) => token,
        };
        let offset = token.text_range().start();

        // 1. Declaration name (definition site; the `x` in `local x = 1`).
        if let Some(facts) = self.file_facts()
            && let Some(decl) = facts.decl_at_offset(offset)
        {
            return Some(decl.id.clone());
        }

        // 1.5 Member key (definition site: the `x` in `{ x = 1 }` / `@field x` / `@field [1]` / `T.x = v`) -> Member.
        // A name inside a `[expr] = v` key is a use of a binding, not the member key.
        let parent = token.parent()?;
        let kind: LuaSyntaxKind = parent.kind().into();
        if kind != LuaSyntaxKind::NameExpr
            && let Some(facts) = self.file_facts()
            && let Some(member) = facts.member_at_offset(offset)
        {
            return Some(member.id.clone());
        }

        // 2. Index-expression member key (the `x` in `T.x`, including definition sites) -> Member.
        if kind == LuaSyntaxKind::IndexExpr
            && let Some(index_expr) = LuaIndexExpr::cast(parent.clone())
        {
            if let Some(resolved) = self.resolve_member(&index_expr) {
                return resolved.member_id;
            }
            if let Some(member) = self.require_field_member(&index_expr) {
                return Some(member);
            }
        }
        // 2b. String-literal member key (the `"x"` in `T["x"]`) names the same member as `T.x`.
        if kind == LuaSyntaxKind::LiteralExpr
            && let Some(index_expr) = parent.parent().and_then(LuaIndexExpr::cast)
            && let Some(key @ emmylua_parser::LuaIndexKey::String(_)) = index_expr.get_index_key()
            && key.get_range() == Some(token.text_range())
            && let Some(resolved) = self.resolve_member(&index_expr)
        {
            return resolved.member_id;
        }
        // 3. Doc name type (the `Old` in `---@type Old`) -> TypeDef.
        if kind == LuaSyntaxKind::TypeName
            && let Some(name_type) = LuaDocNameType::cast(parent.clone())
            && let Some(name) = name_type.get_name_text()
            && let Some(def) = self.resolve_type_def(&name)
        {
            return Some(def.id);
        }
        // 3b. Name token of `---@class Test.Abc` (parent node DocTagClass; names may be dotted full names).
        if kind == LuaSyntaxKind::DocTagClass
            && let Some(tag) = LuaDocTagClass::cast(parent.clone())
            && let Some(name_token) = tag.get_name_token()
            && let Some(def) = self.resolve_type_def(name_token.get_name_text())
        {
            return Some(def.id);
        }
        // 3c. Name token of `---@alias schema.DiagnosticCode` -> TypeDef.
        if kind == LuaSyntaxKind::DocTagAlias
            && let Some(tag) = LuaDocTagAlias::cast(parent.clone())
            && let Some(name_token) = tag.get_name_token()
            && let Some(def) = self.resolve_type_def(name_token.get_name_text())
        {
            return Some(def.id);
        }
        // 4. Binding use (name expression or vararg literal) -> Decl.
        self.resolve_name(offset)
    }
    /// Syntax location (node / token) -> semantic info (type + declaration identity).
    /// Shared query for LSP features such as hover / semantic_token and checkers.
    pub fn semantic_info(
        &self,
        node_or_token: rowan::NodeOrToken<LuaSyntaxNode, LuaSyntaxToken>,
    ) -> Option<SemanticInfo> {
        let token = match node_or_token {
            rowan::NodeOrToken::Node(node) => node.first_token()?,
            rowan::NodeOrToken::Token(token) => token,
        };
        let offset = token.text_range().start();

        // 1. Declaration name (definition site: `local x` / parameter / function name / for variable) -> Decl.
        if let Some(facts) = self.file_facts()
            && let Some(decl) = facts.decl_at_offset(offset)
        {
            return Some(SemanticInfo {
                typ: self.type_of_decl(&decl.id).unwrap_or(LuaType::Unknown),
                decl: Some(decl.id.clone()),
            });
        }

        // 2. Member key (the `x` in table field `{ x = 1 }` / `@field x` / `T.x = v` at definition sites) -> Member.
        if let Some(facts) = self.file_facts()
            && let Some(member) = facts.member_at_offset(offset)
        {
            return Some(SemanticInfo {
                typ: self.type_of_member(&member.id).unwrap_or(LuaType::Unknown),
                decl: Some(member.id.clone()),
            });
        }

        let parent = token.parent()?;
        let kind: LuaSyntaxKind = parent.kind().into();

        // 3. Doc name type (the Old/Foo in `---@type Old` / `---@class Foo`) -> TypeDef.
        if kind == LuaSyntaxKind::TypeName
            && let Some(name_type) = LuaDocNameType::cast(parent.clone())
            && let Some(name) = name_type.get_name_text()
            && let Some(def) = self.resolve_type_def(&name)
        {
            return Some(SemanticInfo {
                typ: self.type_def_ref(&def),
                decl: Some(def.id),
            });
        }

        // 3b. Name token of `---@class Test.Abc` (parent node DocTagClass) -> TypeDef.
        if kind == LuaSyntaxKind::DocTagClass
            && let Some(tag) = LuaDocTagClass::cast(parent.clone())
            && let Some(name_token) = tag.get_name_token()
            && let Some(def) = self.resolve_type_def(name_token.get_name_text())
        {
            return Some(SemanticInfo {
                typ: self.type_def_ref(&def),
                decl: Some(def.id),
            });
        }
        // 3c. Name token of `---@alias schema.DiagnosticCode` -> TypeDef.
        if kind == LuaSyntaxKind::DocTagAlias
            && let Some(tag) = LuaDocTagAlias::cast(parent.clone())
            && let Some(name_token) = tag.get_name_token()
            && let Some(def) = self.resolve_type_def(name_token.get_name_text())
        {
            return Some(SemanticInfo {
                typ: self.type_def_ref(&def),
                decl: Some(def.id),
            });
        }

        // 4. Index-expression member key (the `x` in `T.x` at use sites) -> Member (falls back to the expression type if not found).
        if kind == LuaSyntaxKind::IndexExpr
            && let Some(index_expr) = LuaIndexExpr::cast(parent.clone())
            && let Some(resolved) = self.resolve_member(&index_expr)
            && let Some(member_id) = resolved.member_id
        {
            // A generic instance prefix (`c.aaa` with `local c ---@type MyTable<int>` and `@field aaa T`)
            // must report the projected member type (`integer`), not the declaration-site `T`.
            let typ = self
                .project_member_type_of_generic_prefix(&index_expr, &member_id)
                .or_else(|| self.type_of_member(&member_id))
                .unwrap_or(LuaType::Unknown);
            return Some(SemanticInfo {
                typ,
                decl: Some(member_id),
            });
        }

        // 5. Expression -> type and declaration identity for binding uses.
        if let Some(expr) = LuaExpr::cast(parent) {
            let decl = self.resolve_name(offset);
            let typ = if matches!(&expr, LuaExpr::NameExpr(_)) {
                self.assignment_target_value_type(&expr)
                    .unwrap_or_else(|| self.type_of_expr(expr.get_syntax_id()))
            } else {
                self.type_of_expr(expr.get_syntax_id())
            };
            return Some(SemanticInfo { typ, decl });
        }
        None
    }
    /// Hover type for assignment target `x = value`: when the right-hand type is known and non-nil,
    /// show the actual type after assignment (`x = create()` no longer shows the old `T?`).
    pub(crate) fn assignment_target_value_type(&self, expr: &LuaExpr) -> Option<LuaType> {
        let LuaExpr::NameExpr(_) = expr else {
            return None;
        };
        let assign = expr.syntax().parent().and_then(LuaAssignStat::cast)?;
        let (vars, values) = assign.get_var_and_expr_list();
        let idx = vars
            .iter()
            .position(|var| var.to_expr().get_syntax_id() == expr.get_syntax_id())?;
        // A trailing call fills every remaining target with its returns (`b, c = two()`).
        if let Some(LuaExpr::CallExpr(call)) = values.last()
            && idx + 1 >= values.len()
        {
            let call_ty = self.type_of_expr(call.get_syntax_id());
            let ty = call_ty.get_result_slot_type(idx + 1 - values.len())?;
            return (!matches!(ty, LuaType::Unknown | LuaType::Nil)).then_some(ty);
        }
        let value = values.get(idx)?;
        // Pure name references `x = y` keep declaration/literal display (hover still shows `integer` rather than widened `number`);
        // Assignments that clearly produce a new value (calls/constructors) use the RHS type (`x = create()` removes the old `T?`).
        if matches!(value, LuaExpr::NameExpr(_)) {
            return None;
        }
        let ty = self.type_of_expr(value.get_syntax_id());
        (!matches!(ty, LuaType::Unknown | LuaType::Nil)).then_some(ty)
    }
    /// Projects a member declaration type through the index expression's prefix generic arguments:
    /// `local c ---@type MyTable<int>` + `---@field aaa T` -> `c.aaa: integer`.
    /// Returns `None` when the prefix is not an instantiated generic (or the member type is unknown),
    /// so callers keep the declaration-site member type in every other case.
    fn project_member_type_of_generic_prefix(
        &self,
        index_expr: &LuaIndexExpr,
        member_id: &SemanticId,
    ) -> Option<LuaType> {
        let prefix = index_expr.get_prefix_expr()?;
        let prefix_ty = self.prefix_type_for_member_resolution(&prefix);
        let LuaType::Generic(generic) = &prefix_ty else {
            return None;
        };
        let member_type = self.type_of_member(member_id)?;
        let names = self
            .type_def_of(generic.get_base_type_id_ref())
            .map(|def| {
                def.generic_params
                    .iter()
                    .map(|param| param.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Some(self.substitute_generic_params_named(&member_type, generic.get_params(), &names))
    }
    /// Whether a syntax node references the given declaration (convenience wrapper, equivalent to `is_reference_to(NodeOrToken::Node)`).
    pub fn is_reference_to_syntax(&self, node: &LuaSyntaxNode, decl: &SemanticId) -> bool {
        self.is_reference_to(rowan::NodeOrToken::Node(node.clone()), decl)
    }
    /// Whether a syntax node can access the given declaration (convenience wrapper).
    pub fn is_visible_syntax(&self, node: &LuaSyntaxNode, decl: &SemanticId) -> bool {
        self.is_visible(rowan::NodeOrToken::Node(node.clone()), decl)
    }
    /// Whether a syntax location (node / token) references the given declaration (references / rename / highlight scenarios).
    pub fn is_reference_to(
        &self,
        node_or_token: rowan::NodeOrToken<LuaSyntaxNode, LuaSyntaxToken>,
        decl: &SemanticId,
    ) -> bool {
        let Some(info) = self.semantic_info(node_or_token) else {
            return false;
        };
        info.decl.as_ref() == Some(decl)
    }
    /// Whether a syntax location (node / token) can access the given declaration (visibility check).
    /// M0: Public/Internal -> visible; Package/Private/Protected -> visible within the same file (intra-class access refinement left for later).
    pub fn is_visible(
        &self,
        node_or_token: rowan::NodeOrToken<LuaSyntaxNode, LuaSyntaxToken>,
        decl: &SemanticId,
    ) -> bool {
        let _ = node_or_token;
        // Declaring file (visibility is determined by the declaring file).
        let decl_file = match decl {
            SemanticId::Decl(key) => key.file_id,
            SemanticId::Member(key) => key.file_id,
            SemanticId::TypeDef(key) => match key.scope {
                TypeScope::File(file_id) => file_id,
                _ => return true, // Global types are always visible
            },
            _ => return true,
        };
        // Member visibility annotation.
        if let SemanticId::Member(_) = decl
            && let Some(facts) = self.file_facts_of(decl_file)
            && let Some(member) = facts.member_by_id(decl)
        {
            use VisibilityKind;
            return match member.visibility {
                VisibilityKind::Public | VisibilityKind::Internal => true,
                VisibilityKind::Package | VisibilityKind::Private | VisibilityKind::Protected => {
                    decl_file == self.view.file_id()
                }
            };
        }
        // Type definitions: File scope (@private) is visible only in the same file.
        if let SemanticId::TypeDef(_) = decl {
            return decl_file == self.view.file_id();
        }
        true
    }
    /// Assembles `ResolvedMember` (fills type/visibility/method flag).
    pub(crate) fn resolved_member(
        &self,
        member_id: Option<SemanticId>,
        file_id: Option<FileId>,
        owner: SemanticId,
        name: SmolStr,
    ) -> ResolvedMember {
        let member_id = member_id.map(|id| self.version_visible_sibling(&id).unwrap_or(id));
        let mut member_type = None;
        let mut visibility = None;
        let mut is_method = false;
        if let (Some(member_id), Some(file_id)) = (&member_id, file_id)
            && let Some(facts) = self.file_facts_of(file_id)
            && let Some(member) = facts.member_by_id(member_id)
        {
            // Type is computed lazily (avoids recursion when resolve_member is reverse-called by type_of_member);
            // Consumers can query type_of_member later when they need the full type.
            member_type = None;
            visibility = Some(member.visibility);
            if let Some(value_syntax) = member.value_syntax
                && let Some(signature) = facts.signature_by_closure(value_syntax)
            {
                is_method = signature.is_method;
            }
        }
        ResolvedMember {
            member_id,
            file_id,
            owner,
            name,
            member_type,
            visibility,
            is_method,
        }
    }
    /// Gets the prefix type during member resolution: NameExpr goes directly through non-flow `type_of_decl`,
    /// avoiding recursive explosion from the VM's `type_of_expr` -> `type_of_decl_at` during flow backtracking.
    pub(crate) fn prefix_type_for_member_resolution(&self, expr: &LuaExpr) -> LuaType {
        if let LuaExpr::NameExpr(name_expr) = expr {
            if let Some(decl) = self.resolve_name(name_expr.get_position()) {
                let mut ty = self.type_of_decl(&decl).unwrap_or(LuaType::Unknown);
                // `type_of_decl` may return Unknown for parameters with no call-site inference;
                // member resolution needs the annotated parameter type (the `p._cfg` for `---@param p T2`).
                if matches!(ty, LuaType::Unknown)
                    && let Some(param_ty) = self.param_type_for_decl(&decl)
                {
                    ty = param_ty;
                }
                // `local e` assigned later (`e = Entry.new()`) has no declared type; the type at
                // the use comes from the assignments reaching it.
                if matches!(ty, LuaType::Unknown) && self.is_uninitialized_plain_local(&decl) {
                    let offset = name_expr.get_position();
                    if self.is_read_in_nested_closure(&decl, offset) {
                        // Read in a closure: the flow type has no say, only assignments in the
                        // functions around the read count.
                        let info = self.file_facts().and_then(|facts| facts.decl_by_id(&decl));
                        return info
                            .and_then(|info| self.infer_assigned_type(info, Some(offset)))
                            .unwrap_or(LuaType::Unknown);
                    }
                    return self.type_of_expr(expr.get_syntax_id());
                }
                // `local x = nil` set later (`x = made`): the expression's own type carries the
                // assignments that reach this use.
                if matches!(ty, LuaType::Unknown | LuaType::Nil) {
                    let expr_ty = self.type_of_expr(expr.get_syntax_id());
                    if !matches!(expr_ty, LuaType::Unknown | LuaType::Nil) {
                        ty = expr_ty;
                    }
                }
                return self.attach_param_decl_constraint(&decl, ty);
            }
        }
        self.type_of_expr(expr.get_syntax_id())
    }
    /// Takes the `---@param` annotation type directly from the parameter's owning closure (skips `type_of_decl`).
    pub(crate) fn param_type_for_decl(&self, decl: &SemanticId) -> Option<LuaType> {
        let SemanticId::Decl(decl_key) = decl else {
            return None;
        };
        let facts = self.file_facts_of(decl_key.file_id)?;
        let decl_info = facts.decl_by_id(decl)?;
        if !matches!(decl_info.kind, DeclKind::Param) {
            return None;
        }
        let closure_syntax = decl_info.owner_syntax?;
        let signature = facts
            .signatures
            .iter()
            .find(|sig| sig.closure_syntax == closure_syntax)?;
        let param_index = signature
            .param_names
            .iter()
            .position(|name| name == &decl_info.name)?;
        self.param_type(closure_syntax, param_index)
    }
    /// Whether the union member is missing from some components: when `A|C` has `handle` only on A,
    /// `target.handle` should report a missing member in parameter checks rather than silently taking A's field type.
    pub fn member_missing_in_union(&self, index_expr: &LuaIndexExpr) -> bool {
        let Some(index_key) = index_expr.get_index_key() else {
            return false;
        };
        let Some(prefix) = index_expr.get_prefix_expr() else {
            return false;
        };
        let prefix_ty = self.prefix_type_for_member_resolution(&prefix);
        let LuaType::Union(union) = &prefix_ty else {
            return false;
        };
        let key = LuaMemberKey::Name(SmolStr::new(index_key.get_path_part()));
        union
            .into_vec()
            .iter()
            .any(|component| self.member_type(component, &key).is_none())
    }
    /// Resolves member references in index expressions (single member resolution entry point):
    /// same-file member -> same-file class `@field` -> cross-file runtime member (resolve owner -> merge members).
    /// Cached `callable_functions` result for a callee type.
    pub(crate) fn callable_functions_cached(&self, ty: &LuaType) -> Vec<LuaFunctionType> {
        if let Some(cached) = self.cache.borrow().callable_functions.get(ty) {
            return cached.clone();
        }
        let value = callable_functions(self, ty);
        self.cache
            .borrow_mut()
            .callable_functions
            .insert(ty.clone(), value.clone());
        value
    }
    /// Unified callable candidates for a name callee.
    ///
    /// `f` may be declared in multiple files (global overloads); this entry goes
    /// through the same `signature_candidates` projection used by the VM, so
    /// diagnostics/completion see the same set.
    pub(crate) fn callable_candidates_for_name_expr(
        &self,
        callee: &LuaExpr,
    ) -> Vec<LuaFunctionType> {
        let LuaExpr::NameExpr(name_expr) = callee else {
            return Vec::new();
        };
        let Some(owner) = self.resolve_name(name_expr.get_position()) else {
            return Vec::new();
        };
        // Only intervene for actual same-workspace global overloads. A single
        // declaration keeps the legacy projection path, which preserves its
        // generic/closure details.
        let SemanticId::Decl(key) = &owner else {
            return Vec::new();
        };
        let Some(facts) = self.file_facts_of(key.file_id) else {
            return Vec::new();
        };
        let Some(decl) = facts.decl_by_id(&owner) else {
            return Vec::new();
        };
        if !matches!(decl.kind, DeclKind::Global) {
            return Vec::new();
        }
        let decls = self.global_decls_named_for_file(key.file_id, decl.name.as_str());
        if decls.len() <= 1 {
            return Vec::new();
        }
        let vm = infer::vm::InferVm::new(self, &[]);
        // Identical duplicate declarations (same file content loaded twice, common in
        // tests / project duplicates) must keep the legacy single-symbol path; only
        // genuinely different global declarations are overloads.
        let mut per_decl: Vec<Vec<LuaFunctionType>> = Vec::with_capacity(decls.len());
        for decl in &decls {
            let candidates = vm
                .callable_candidates_for_owner_single(decl)
                .unwrap_or_default();
            per_decl.push(candidates);
        }
        let Some(first) = per_decl.first() else {
            return Vec::new();
        };
        if per_decl.iter().all(|candidates| candidates == first) {
            return Vec::new();
        }
        per_decl.into_iter().flatten().collect()
    }

    /// Public unified candidates for call rendering (signature help / completion / hover).
    ///
    /// Combines the diagnostic candidate path (member `CallableCandidateSet`,
    /// cross-file globals) with the full signature projection (main + `---@overload`)
    /// so all rendering paths share one candidate list.
    pub fn callable_candidates_for_expr(&self, callee: &LuaExpr) -> Vec<LuaFunctionType> {
        let mut out = callable_candidates_uncached(self, callee);
        // The diagnostic path may only project the callee value type; append the
        // full signature candidates (main + `---@overload`) for name callees.
        if let LuaExpr::NameExpr(name_expr) = callee
            && let Some(owner) = self.resolve_name(name_expr.get_position())
        {
            let vm = infer::vm::InferVm::new(self, &[]);
            if let Some(candidates) = vm.callable_candidates_for_owner(&owner) {
                for candidate in candidates {
                    if !out.contains(&candidate) {
                        out.push(candidate);
                    }
                }
            }
        }
        out
    }

    pub(crate) fn callable_candidates_cached(&self, callee: &LuaExpr) -> Vec<LuaFunctionType> {
        let syntax = callee.get_syntax_id();
        let file_id = self.view.file_id();
        if let Some(cached) = self
            .cache
            .borrow()
            .callable_candidates
            .get(&(file_id, syntax))
        {
            return cached.clone();
        }
        let value = callable_candidates_uncached(self, callee);
        self.cache
            .borrow_mut()
            .callable_candidates
            .insert((file_id, syntax), value.clone());
        value
    }
    pub(crate) fn call_site_analysis(&self, call_expr: &LuaCallExpr) -> CallSiteAnalysis {
        let syntax = call_expr.get_syntax_id();
        let file_id = self.view.file_id();
        if let Some(cached) = self.cache.borrow().call_site.get(&(file_id, syntax)) {
            return cached.clone();
        }
        let analysis = self.call_site_analysis_uncached(call_expr);
        self.cache
            .borrow_mut()
            .call_site
            .insert((file_id, syntax), analysis.clone());
        analysis
    }
    pub(crate) fn call_site_analysis_uncached(&self, call_expr: &LuaCallExpr) -> CallSiteAnalysis {
        let Some(callee) = call_expr.get_prefix_expr() else {
            return CallSiteAnalysis {
                candidates: Vec::new(),
                arg_types: Vec::new(),
                colon_call: call_expr.is_colon_call(),
                receiver_ty: LuaType::Unknown,
                explicit_generics: Vec::new(),
            };
        };
        let candidates = self.callable_candidates_cached(&callee);
        if candidates.is_empty() {
            return CallSiteAnalysis {
                candidates,
                arg_types: Vec::new(),
                colon_call: call_expr.is_colon_call(),
                receiver_ty: LuaType::Unknown,
                explicit_generics: Vec::new(),
            };
        }
        let args = call_expr
            .get_args_list()
            .map(|list| list.get_args().collect::<Vec<_>>())
            .unwrap_or_default();
        let arg_types: Vec<LuaType> = args
            .iter()
            .map(|arg| self.type_of_expr(arg.get_syntax_id()))
            .collect();
        let colon_call = call_expr.is_colon_call();
        let receiver_ty = if colon_call {
            LuaIndexExpr::cast(callee.syntax().clone())
                .and_then(|index| index.get_prefix_expr())
                .map(|prefix| self.type_of_expr(prefix.get_syntax_id()))
                .unwrap_or(LuaType::Unknown)
        } else {
            LuaType::Unknown
        };
        let explicit_generics: Vec<LuaSyntaxId> = call_expr
            .get_call_generic_type_list()
            .map(|list| list.get_types().map(|ty| ty.get_syntax_id()).collect())
            .unwrap_or_default();
        // Resolved signatures are intentionally lazy: they are only needed by the
        // generic-constraint checker. Computing them eagerly for every call is one of
        // the largest costs in `ParamTypeChecker`.
        CallSiteAnalysis {
            candidates,
            arg_types,
            colon_call,
            receiver_ty,
            explicit_generics,
        }
    }
    /// Lazily compute and cache resolved call signatures (generic bindings per candidate).
    pub(crate) fn call_site_signatures(
        &self,
        call_expr: &LuaCallExpr,
    ) -> Vec<(LuaFunctionType, infer::unify::TplBindings)> {
        let syntax = call_expr.get_syntax_id();
        let file_id = self.view.file_id();
        if let Some(cached) = self
            .cache
            .borrow()
            .call_site_signatures
            .get(&(file_id, syntax))
        {
            return cached.clone();
        }
        let analysis = self.call_site_analysis(call_expr);
        let signatures = resolved_call_signatures(
            self,
            call_expr,
            &analysis.candidates,
            &analysis.arg_types,
            analysis.colon_call,
            &analysis.receiver_ty,
        );
        self.cache
            .borrow_mut()
            .call_site_signatures
            .insert((file_id, syntax), signatures.clone());
        signatures
    }
    /// Root variable and dotted key path of an access like `vim.fn.stdpath` or `h.check`, the
    /// identity under which its uses are linked when the field is declared nowhere.
    pub fn unresolved_member_key(
        &self,
        index_expr: &LuaIndexExpr,
    ) -> Option<(SemanticId, Vec<SmolStr>)> {
        crate::semantic_db::query::unresolved_member_key(self.file_facts()?, index_expr)
    }
    pub fn resolve_member(&self, index_expr: &LuaIndexExpr) -> Option<ResolvedMember> {
        let syntax = index_expr.get_syntax_id();
        let file_id = self.view.file_id();
        if let Some(cached) = self.cache.borrow().resolve_member.get(&(file_id, syntax)) {
            return cached.clone();
        }
        let result = self.resolve_member_impl(index_expr);
        self.cache
            .borrow_mut()
            .resolve_member
            .insert((file_id, syntax), result.clone());
        result
    }
    pub(crate) fn resolve_member_impl(&self, index_expr: &LuaIndexExpr) -> Option<ResolvedMember> {
        // A receiver that is not a name chain (`open():close()`, `lazy().f`) has no syntactic owner;
        // only the receiver's type can find the member, so use a placeholder owner that matches nothing.
        let (owner, name) = match self
            .analysis()
            .member_ref_of_index(self.view.file_id(), index_expr.get_syntax_id())
        {
            Some(owner_and_name) => owner_and_name,
            None => {
                // `require("m").f` is resolved through the module's exports instead.
                if let Some(LuaExpr::CallExpr(call)) = index_expr.get_prefix_expr()
                    && call.is_require()
                {
                    let name = SmolStr::new(index_expr.get_index_key()?.get_path_part());
                    let module_owner = self.require_module_owner(index_expr)?;
                    let member_ref = self
                        .members_of_owner_named(&module_owner, name.as_str())
                        .first()
                        .cloned()?;
                    return Some(self.resolved_member(
                        Some(member_ref.id.clone()),
                        Some(member_ref.file_id),
                        module_owner,
                        name,
                    ));
                }
                (
                    SemanticId::name(SmolStr::new(EXPR_OWNER)),
                    SmolStr::new(index_expr.get_index_key()?.get_path_part()),
                )
            }
        };
        let is_expr_owner = matches!(&owner, SemanticId::Name(name) if name.as_str() == EXPR_OWNER);

        // 1. Same-file members (owner key). Members with explicit `---@type` take priority over purely inferred runtime members;
        // if only runtime members exist and the prefix type is a named type with a same-named `@field`, skip this step --
        // under `---@param data CreateData`, `data.owner = ""` should resolve to the class field rather than this assignment.
        let use_path = self.local_chain_path(&owner, index_expr);
        {
            let facts = self.file_facts()?;
            let same_name = facts
                .members_of_owner_named(&owner, name.as_str())
                .filter(|member| self.declared_on_path(&member.id, use_path.as_deref()))
                .collect::<Vec<_>>();
            let prefer_typed = same_name
                .iter()
                .all(|member| member.doc_type_syntax.is_none())
                && (self.prefix_has_named_member(index_expr, &name)
                    || self.require_module_has_member(index_expr, &name));
            if !prefer_typed {
                let member_id = same_name
                    .iter()
                    .find(|member| member.doc_type_syntax.is_some())
                    .or_else(|| same_name.first())
                    .map(|member| member.id.clone());
                if let Some(member_id) = member_id {
                    return Some(self.resolved_member(
                        Some(member_id),
                        Some(self.view.file_id()),
                        owner,
                        name,
                    ));
                }
            }
        }

        // 2. Same-file class definition: `@field` for `C.field`.
        if let Some(facts) = self.file_facts()
            && let Some(LuaExpr::NameExpr(prefix)) = index_expr.get_prefix_expr()
            && let Some(prefix_text) = prefix.get_name_text()
            && let Some(def) = facts.type_def_by_name(&prefix_text)
            && let Some(member_id) = facts
                .field_members_of_type(&def.id, &name)
                .map(|member| member.id.clone())
        {
            return Some(self.resolved_member(
                Some(member_id),
                Some(self.view.file_id()),
                owner,
                name,
            ));
        }

        // Compute the prefix type once for the remaining slow-path stages. `resolve_member`
        // must not repeatedly infer the same prefix expression (3 / 3.5 / inherited fallback).
        // `nil` has no members: a `T|nil` receiver (`local f = io.open(p)`) resolves through `T`.
        let prefix_ty = index_expr.get_prefix_expr().map(|prefix| {
            super::types::remove_nil_from_type(self.prefix_type_for_member_resolution(&prefix))
        });

        // 3. Prefix type is a named type -> its `@field` members (cross-file, e.g. `c.secret` where c: C).
        //    Also look up the runtime value declaration for that type (`---@class Game` + `local Game = {}`),
        //    otherwise dot access like `game.add` cannot resolve `function Game:add()`.
        if let Some(prefix_ty) = &prefix_ty {
            let type_id = match prefix_ty {
                LuaType::Ref(id) | LuaType::Def(id) => Some(id),
                LuaType::Generic(generic) => Some(generic.get_base_type_id_ref()),
                LuaType::TplRef(tpl) => match tpl.get_constraint() {
                    Some(LuaType::Ref(id)) | Some(LuaType::Def(id)) => Some(id),
                    Some(LuaType::Generic(generic)) => Some(generic.get_base_type_id_ref()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(id) = type_id
                && let Some(def) = self.type_def_of(id)
            {
                // Class types also consider runtime implementations (`---@class Game` + `local Game = {}` +
                // `function Game:add()`); enums/aliases only look at the `@field` surface to avoid
                // treating fields in enum implementations as valid members.
                let owners = if def.kind == TypeDefKind::Class {
                    self.analysis().resolve_owner_set(def.id.clone())
                } else {
                    vec![def.id.clone()]
                };
                for resolved in owners {
                    if let Some(member_ref) = self
                        .members_of_owner_named(&resolved, name.as_str())
                        .first()
                    {
                        return Some(self.resolved_member(
                            Some(member_ref.id.clone()),
                            Some(member_ref.file_id),
                            owner,
                            name,
                        ));
                    }
                    // Runtime members from `self.x = ...` in method bodies: attach members on the implicit self parameter
                    // to the class/runtime owner of that method (fields assigned in `function T:init`,
                    // then `p.x` from a `T` instance should resolve).
                    for method_ref in self.members_of_owner(&resolved).iter() {
                        let Some(method_facts) = self.file_facts_of(method_ref.file_id) else {
                            continue;
                        };
                        let Some(method_member) = method_facts.member_by_id(&method_ref.id) else {
                            continue;
                        };
                        let Some(closure_syntax) = method_member.value_syntax else {
                            continue;
                        };
                        let Some(self_decl) = method_facts
                            .decls
                            .iter()
                            .find(|d| d.name == "self" && d.owner_syntax == Some(closure_syntax))
                        else {
                            continue;
                        };
                        let self_members = method_facts
                            .members_of_owner_named(&self_decl.id, name.as_str())
                            .collect::<Vec<_>>();
                        if let Some(self_member_ref) = self_members.into_iter().next() {
                            let file_id = match &self_member_ref.id {
                                SemanticId::Member(key) => Some(key.file_id),
                                _ => None,
                            };
                            return Some(self.resolved_member(
                                Some(self_member_ref.id.clone()),
                                file_id,
                                owner,
                                name,
                            ));
                        }
                    }
                }
                // Alias types have no `@field` surface of their own. Resolve through the alias
                // target so `---@type SomeAlias` values can still find class/table members.
                if def.kind == TypeDefKind::Alias
                    && let Some(info) =
                        self.member_info(prefix_ty, &LuaMemberKey::Name(name.clone()))
                    && let Some(member_id) = info.id
                {
                    let file_id = match &member_id {
                        SemanticId::Member(key) => Some(key.file_id),
                        _ => None,
                    };
                    return Some(self.resolved_member(Some(member_id), file_id, owner, name));
                }
            }
        }

        // 3.5 Prefix type is an anonymous table (TableConst) -> its table-field members.
        //     Handles the `y` in `T.x.y` after `local T = { x = { y = 1 } }`.
        //     A module's type is its class plus its table, so a union is searched for its table too.
        let table_const = match prefix_ty.as_ref() {
            Some(LuaType::TableConst(table)) => Some(table.clone()),
            Some(LuaType::Union(union)) => {
                let mut tables =
                    union
                        .into_vec()
                        .into_iter()
                        .filter_map(|component| match component {
                            LuaType::TableConst(table) => Some(table),
                            _ => None,
                        });
                match (tables.next(), tables.next()) {
                    (Some(table), None) => Some(table),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(in_field) = table_const.as_ref() {
            let table_owner = SemanticId::member(in_field.file_id, in_field.value);
            let mut table_owners = vec![table_owner];
            // TableConst for named local tables (`local checker = { ... }`) must also resolve
            // members like `function checker:is_player()`, not only the synthetic owner of anonymous tables.
            if let Some(decl) = self
                .file_facts_of(in_field.file_id)
                .and_then(|facts| facts.decl_by_value_range(in_field.value))
            {
                table_owners.push(decl.id.clone());
            }
            // A literal nested in another (`tester = { test = {} }`): the members that
            // `function tester.test.f()` declares sit under the field or the root table.
            if let Some(facts) = self.file_facts_of(in_field.file_id) {
                for kind in [
                    LuaSyntaxKind::TableEmptyExpr,
                    LuaSyntaxKind::TableObjectExpr,
                    LuaSyntaxKind::TableArrayExpr,
                ] {
                    if let Some(member) =
                        facts.member_by_value_syntax(LuaSyntaxId::new(kind.into(), in_field.value))
                    {
                        table_owners.push(member.id.clone());
                        table_owners.push(member.owner.clone());
                    }
                }
            }
            for table_owner in table_owners {
                if let Some(member) = self
                    .members_of_owner_named(&table_owner, name.as_str())
                    .first()
                {
                    return Some(self.resolved_member(
                        Some(member.id.clone()),
                        Some(member.file_id),
                        owner,
                        name,
                    ));
                }
                // Runtime members defined by `self.x = ...` in that table's method bodies are also attached to the table.
                for method_ref in self.members_of_owner(&table_owner).iter() {
                    let Some(method_facts) = self.file_facts_of(method_ref.file_id) else {
                        continue;
                    };
                    let Some(method_member) = method_facts.member_by_id(&method_ref.id) else {
                        continue;
                    };
                    if !method_member.is_method {
                        continue;
                    }
                    let Some(closure_syntax) = method_member.value_syntax else {
                        continue;
                    };
                    let Some(self_decl) = method_facts
                        .decls
                        .iter()
                        .find(|d| d.name == "self" && d.owner_syntax == Some(closure_syntax))
                    else {
                        continue;
                    };
                    let self_members = method_facts
                        .members_of_owner_named(&self_decl.id, name.as_str())
                        .collect::<Vec<_>>();
                    if let Some(self_member_ref) = self_members.into_iter().next() {
                        let file_id = match &self_member_ref.id {
                            SemanticId::Member(key) => Some(key.file_id),
                            _ => None,
                        };
                        return Some(self.resolved_member(
                            Some(self_member_ref.id.clone()),
                            file_id,
                            owner,
                            name,
                        ));
                    }
                }
            }
        }

        // 4. Cross-file runtime members + require module export members.
        //    Expand multiple identities locally (avoids dependency cycles between tracked resolve_owner_set and type_of_member):
        //    Old-path owners take priority; non-public members or members with doc declarations win by score.
        let old_owner = self.resolve_owner(&owner);
        // P5b: candidate owners come from deterministic canonical owner
        // resolution instead of ad-hoc type/global/alias heuristics.
        let mut cross_owners: Vec<SemanticId> = Vec::new();
        for owner_id in self.analysis().resolve_owner_ids(&owner) {
            if let Some(raw) = self.analysis().owner_id_to_semantic_id(&owner_id)
                && !cross_owners.contains(&raw)
            {
                cross_owners.push(raw);
            }
        }
        // Keep the raw owner itself for members stored under `Name` / `Decl` /
        // `Member` keys (global runtime members, local table members).
        if !cross_owners.contains(&owner) {
            cross_owners.push(owner.clone());
        }
        if let Some(module_owner) = self.require_module_owner(index_expr)
            && !cross_owners.contains(&module_owner)
        {
            cross_owners.push(module_owner);
        }
        // A value typed by `---@module "m"` (a lazily required table field): the module's members.
        if let Some(LuaType::ModuleRef(module_file)) = &prefix_ty
            && let Some(module_owner) = self.module_export_owner_of(*module_file)
            && !cross_owners.contains(&module_owner)
        {
            cross_owners.push(module_owner);
        }
        // `self` of `function Reader:m()` where `Reader` has no type (or only the anonymous table
        // a class factory returns): the methods declared on `Reader` are the members of `self`.
        if prefix_ty.as_ref().is_none_or(|ty| {
            matches!(
                ty,
                LuaType::Unknown | LuaType::Table | LuaType::TableConst(_)
            )
        })
            && let Some(method_owner) = self.implicit_self_method_owner(&owner)
            && !cross_owners.contains(&method_owner)
        {
            cross_owners.push(method_owner);
        }
        // The fields `self.x = v` assigns in one method are fields of `self` in every other method of the same
        // table, also when the table is built by a call (`local C = class()`) and so has no literal to search.
        if matches!(owner, SemanticId::Decl(_))
            && !matches!(
                prefix_ty,
                Some(LuaType::Ref(_) | LuaType::Def(_) | LuaType::Generic(_))
            )
        {
            for related in crate::semantic_db::query::self_related_owners(self.view.db(), &owner) {
                if !cross_owners.contains(&related) {
                    cross_owners.push(related);
                }
            }
        }
        // Explicit, deterministic preference order (replaces the previous
        // numeric score). The key is intentionally orderable, not weighted:
        //   1. non-resolved owners before the raw owner (`old_owner`);
        //   2. non-public members before public ones;
        //   3. documented members before inferred ones;
        //   4. source/discovery order.
        // This preserves the historical score outcome while making the rule
        // inspectable and easy to change in P6.
        let mut best: Option<((u8, u8, u8, usize), MemberRef)> = None;
        let mut candidate_index = 0usize;
        for resolved in &cross_owners {
            let owner_rank = if old_owner.as_ref() == Some(resolved) {
                1
            } else {
                0
            };
            for member in self.members_of_owner(resolved).iter() {
                if member.name != name {
                    continue;
                }
                // Only the members owned by the root itself are told apart by path; another owner
                // (a module, a type) was reached through the receiver's type.
                if resolved == &owner && !self.declared_on_path(&member.id, use_path.as_deref()) {
                    continue;
                }
                let mut visibility_rank = 1u8;
                let mut doc_rank = 1u8;
                if let Some(facts) = self.file_facts_of(member.file_id)
                    && let Some(member_facts) = facts.member_by_id(&member.id)
                {
                    if member_facts.visibility != VisibilityKind::Public {
                        visibility_rank = 0;
                    }
                    if member_facts.doc_type_syntax.is_some() {
                        doc_rank = 0;
                    }
                }
                let key = (owner_rank, visibility_rank, doc_rank, candidate_index);
                candidate_index += 1;
                if best.as_ref().is_none_or(|(best_key, _)| key < *best_key) {
                    best = Some((key, member.clone()));
                }
            }
        }
        if let Some((_, member)) = best {
            return Some(self.resolved_member(Some(member.id), Some(member.file_id), owner, name));
        }

        // Inherited/cross-class members: `member_info` projects along parent types, enabled only for class types;
        // same-name member resolution on unions is left to flow narrowing to avoid breaking dynamic field tests on `Foo|Bar`.
        if let Some(prefix_ty) = &prefix_ty {
            let is_class = match prefix_ty {
                LuaType::Ref(id) | LuaType::Def(id) => self
                    .type_def_of(id)
                    .is_some_and(|def| def.kind == TypeDefKind::Class),
                LuaType::Generic(generic) => self
                    .type_def_of(generic.get_base_type_id_ref())
                    .is_some_and(|def| def.kind == TypeDefKind::Class),
                LuaType::Union(_) | LuaType::Unknown | LuaType::Any | LuaType::Nil => false,
                _ => true,
            };
            if is_class {
                let key = LuaMemberKey::Name(name.clone());
                if let Some(info) = self.member_info(prefix_ty, &key)
                    && let Some(id) = info.id
                    && let Some(file_id) = info.file_id
                {
                    return Some(self.resolved_member(Some(id), Some(file_id), owner, name));
                }
            }
        }

        // Anonymous table whose metatable `__index` is a class table (`setmetatable({}, M)`).
        if let Some(prefix_ty @ LuaType::TableConst(_)) = &prefix_ty
            && let Some(info) = self.member_info(prefix_ty, &LuaMemberKey::Name(name.clone()))
            && let Some(id) = info.id
            && let Some(file_id) = info.file_id
        {
            return Some(self.resolved_member(Some(id), Some(file_id), owner, name));
        }

        // `local api = vim.api; api.f`: look the member up on the aliased global path.
        if let Some(path_owner) = self.aliased_path_owner(index_expr)
            && let Some(member) = self
                .members_of_owner_named(&path_owner, name.as_str())
                .first()
        {
            return Some(self.resolved_member(
                Some(member.id.clone()),
                Some(member.file_id),
                owner,
                name,
            ));
        }

        // A call result typed as a union (`require("m")` is the module class plus its table) has no
        // flow narrowing to defer to, so take the first component that has the member.
        if is_expr_owner && let Some(LuaType::Union(union)) = &prefix_ty {
            let key = LuaMemberKey::Name(name.clone());
            for component in union.into_vec() {
                if let Some(info) = self.member_info(&component, &key)
                    && let Some(id) = info.id
                    && let Some(file_id) = info.file_id
                {
                    return Some(self.resolved_member(Some(id), Some(file_id), owner, name));
                }
            }
        }

        // A named receiver (`opts`) whose type is a union where exactly one component has the member
        // (`Opts|fun()|table`): the member is that one's.
        if let Some(LuaType::Union(union)) = &prefix_ty {
            let key = LuaMemberKey::Name(name.clone());
            let mut found: Vec<(SemanticId, FileId)> = Vec::new();
            for component in union.into_vec() {
                if let Some(info) = self.member_info(&component, &key)
                    && let Some(id) = info.id
                    && let Some(file_id) = info.file_id
                    && !found.iter().any(|(known, _)| known == &id)
                {
                    found.push((id, file_id));
                }
            }
            if let [(id, file_id)] = found.as_slice() {
                return Some(self.resolved_member(Some(id.clone()), Some(*file_id), owner, name));
            }
        }

        Some(self.resolved_member(None, None, owner, name))
    }
    /// Keys between a local root and the accessed key (`state.client.close` -> `[client]`).
    ///
    /// Members of a local root are owned by the root alone, so `state.client.close` and
    /// `state.handle:close` share an owner; the path tells them apart. `None` when the owner is not
    /// a local or the receiver is not a name chain.
    fn local_chain_path(
        &self,
        owner: &SemanticId,
        index_expr: &LuaIndexExpr,
    ) -> Option<Vec<SmolStr>> {
        if !matches!(owner, SemanticId::Decl(_)) {
            return None;
        }
        index_prefix_path(index_expr)
    }
    /// Whether `member` is declared on `use_path` (see `local_chain_path`). A member that is not
    /// declared through a dotted name (a table-literal field) is kept.
    fn declared_on_path(&self, member: &SemanticId, use_path: Option<&[SmolStr]>) -> bool {
        let Some(use_path) = use_path else {
            return true;
        };
        let SemanticId::Member(key) = member else {
            return true;
        };
        let model = self.model_for(key.file_id);
        let declared = model
            .chunk()
            .and_then(|chunk| {
                chunk
                    .syntax()
                    .token_at_offset(key.key_range.start())
                    .right_biased()
            })
            .and_then(|token| token.parent())
            .and_then(LuaIndexExpr::cast)
            .and_then(|declared| index_prefix_path(&declared));
        declared.is_none_or(|declared| declared == use_path)
    }
    /// Owner named by the initializer of the local that prefixes `index_expr`
    /// (`local api = vim.api` -> the owner of `vim.api` members).
    fn aliased_path_owner(&self, index_expr: &LuaIndexExpr) -> Option<SemanticId> {
        let LuaExpr::NameExpr(prefix) = index_expr.get_prefix_expr()? else {
            return None;
        };
        let decl = self.resolve_name(prefix.get_position())?;
        self.aliased_global_owner(&decl)
    }
    /// Owner of the global path a local is initialised with (`local string = string`,
    /// `local api = vim.api`).
    pub(crate) fn aliased_global_owner(&self, decl: &SemanticId) -> Option<SemanticId> {
        let SemanticId::Decl(key) = decl else {
            return None;
        };
        let facts = self.file_facts_of(key.file_id)?;
        let value = facts.decl_by_id(decl)?.value_expr_syntax?;
        let model = self.model_for(key.file_id);
        let value_node = value.to_node_from_root(model.chunk()?.syntax())?;
        let init = match LuaExpr::cast(value_node)? {
            LuaExpr::IndexExpr(init) => init,
            // `local string = string`: the alias shares the global's members.
            LuaExpr::NameExpr(init) => {
                let global = init.get_name_text()?;
                if model.resolve_local_name(init.get_position()).is_some() {
                    return None;
                }
                return Some(SemanticId::name(SmolStr::new(global)));
            }
            _ => return None,
        };
        let (parent, segment) =
            crate::semantic_db::query::member_ref_from_index_expr(facts, &init)?;
        match parent {
            SemanticId::Name(root) => Some(SemanticId::name(SmolStr::new(format!(
                "{}.{}",
                root.as_str(),
                segment
            )))),
            _ => None,
        }
    }
    /// Owner of the method whose implicit `self` parameter is `owner` (`Reader` for the `self`
    /// of `function Reader:m()`).
    fn implicit_self_method_owner(&self, owner: &SemanticId) -> Option<SemanticId> {
        let SemanticId::Decl(key) = owner else {
            return None;
        };
        let file_model = self.model_for(key.file_id);
        let facts = file_model.file_facts()?;
        let decl = facts.decl_by_id(owner)?;
        if decl.name != "self" {
            return None;
        }
        let closure_syntax = file_model.method_closure_for_self_decl(decl)?;
        Some(facts.member_by_value_syntax(closure_syntax)?.owner.clone())
    }
    /// Whether the prefix type is a named type with a same-named member (avoids runtime inferred members shadowing class `@field`).
    pub(crate) fn prefix_has_named_member(
        &self,
        index_expr: &LuaIndexExpr,
        name: &SmolStr,
    ) -> bool {
        let Some(prefix) = index_expr.get_prefix_expr() else {
            return false;
        };
        let prefix_ty = self.prefix_type_for_member_resolution(&prefix);
        // `W?` and `W|fun()` name the member through the `W` component.
        let components = match &prefix_ty {
            LuaType::Union(union) => union.into_vec(),
            other => vec![other.clone()],
        };
        components.iter().any(|component| {
            let type_id = match component {
                LuaType::Ref(id) | LuaType::Def(id) => Some(id),
                LuaType::Generic(generic) => Some(generic.get_base_type_id_ref()),
                _ => None,
            };
            type_id
                .and_then(|id| self.type_def_of(id))
                .is_some_and(|def| {
                    self.members_of_owner(&def.id)
                        .iter()
                        .any(|member| member.name == *name)
                })
        })
    }
    /// Whether the require module export declares this member (same-name assignments do not shadow exported members).
    pub(crate) fn require_module_has_member(
        &self,
        index_expr: &LuaIndexExpr,
        name: &SmolStr,
    ) -> bool {
        let Some(owner) = self.require_module_owner(index_expr) else {
            return false;
        };
        self.members_of_owner(&owner)
            .iter()
            .any(|member| member.name == *name)
    }
    /// `local M = require("mod")` / `require("mod")` -> module export declaration owner.
    ///
    /// Also follows simple local alias chains (`local N = M`) and parentheses so
    /// every alias form attaches members to the canonical module owner.
    pub(crate) fn require_module_owner(&self, index_expr: &LuaIndexExpr) -> Option<SemanticId> {
        let prefix = index_expr.get_prefix_expr()?;
        self.require_module_member_owner_of_expr(&prefix, 0)
    }

    /// Owner of the table/value a require-rooted expression points to.
    ///
    /// - direct `require("mod")` / alias `M` -> module export owner;
    /// - `M.sub` / `require("mod").sub` -> the `sub` member's table owner,
    ///   so `M.sub.foo` resolves `foo` against the nested table.
    fn require_module_member_owner_of_expr(
        &self,
        expr: &LuaExpr,
        depth: usize,
    ) -> Option<SemanticId> {
        if depth > 8 {
            return None;
        }
        match expr {
            LuaExpr::ParenExpr(paren) => {
                self.require_module_member_owner_of_expr(&paren.get_expr()?, depth + 1)
            }
            LuaExpr::IndexExpr(index_expr) => {
                let (parent, name) = match self
                    .require_module_member_owner_of_expr(&index_expr.get_prefix_expr()?, depth + 1)
                {
                    Some(parent) => (
                        parent,
                        SmolStr::new(index_expr.get_index_key()?.get_path_part()),
                    ),
                    // `vim.lsp = require("vim.lsp")` in another file: the path `vim.lsp` is
                    // not require-rooted, but its assigned member still holds the module.
                    None => self
                        .analysis()
                        .member_ref_of_index(self.view.file_id(), index_expr.get_syntax_id())?,
                };
                let members = self.members_of_owner_named(&parent, name.as_str());
                let member = members.first()?;
                self.member_value_owner_of(&member)
                    .or_else(|| Some(member.id.clone()))
            }
            _ => {
                let module_file = self.require_module_file_of_expr(expr)?;
                self.module_export_owner_of(module_file)
            }
        }
    }

    /// Table/value owner behind a member definition, used for nested require paths.
    fn member_value_owner_of(&self, member: &MemberRef) -> Option<SemanticId> {
        let facts = self.file_facts_of(member.file_id)?;
        let member_def = facts.member_by_id(&member.id)?;
        let value_syntax = member_def.value_syntax?;
        let tree = self.syntax_tree_of(member.file_id)?;
        let node = value_syntax.to_node_from_root(&tree.get_red_root())?;
        let value_expr = LuaExpr::cast(node)?;
        // `util = require 'mod'` / `M.util = require('mod')`: the field holds the required module.
        if matches!(value_expr, LuaExpr::CallExpr(_) | LuaExpr::NameExpr(_)) {
            let member_model = self.model_for(member.file_id);
            if let Some(owner) = member_model
                .require_module_file_of_expr(&value_expr)
                .and_then(|module_file| member_model.module_export_owner_of(module_file))
            {
                return Some(owner);
            }
        }
        match value_expr {
            LuaExpr::TableExpr(table) => {
                Some(SemanticId::member(member.file_id, table.get_range()))
            }
            LuaExpr::NameExpr(name_expr) => {
                let name = name_expr.get_name_text()?;
                let decl =
                    facts.find_visible_decl_before_offset(&name, name_expr.get_position())?;
                Some(decl.id.clone())
            }
            LuaExpr::CallExpr(call) => {
                let model = self.model_for(member.file_id);
                match model
                    .require_module_file_of_call(&call)
                    .and_then(|module_file| model.module_export_owner_of(module_file))
                {
                    Some(owner) => Some(owner),
                    None => Some(member.id.clone()),
                }
            }
            _ => Some(member.id.clone()),
        }
    }

    /// Expression -> required module file.
    ///
    /// Supports `require("mod")`, `local M = require("mod"); M` and
    /// `local N = M` chains (depth-limited).
    pub(crate) fn require_module_file_of_expr(&self, expr: &LuaExpr) -> Option<FileId> {
        self.require_module_file_of_expr_inner(expr, 0)
    }

    fn require_module_file_of_expr_inner(&self, expr: &LuaExpr, depth: usize) -> Option<FileId> {
        if depth > 8 {
            return None;
        }
        match expr {
            LuaExpr::CallExpr(call) => self.require_module_file_of_call(call),
            LuaExpr::ParenExpr(paren) => {
                self.require_module_file_of_expr_inner(&paren.get_expr()?, depth + 1)
            }
            LuaExpr::NameExpr(name_expr) => {
                let decl = self.resolve_name(name_expr.get_position())?;
                let facts = self.file_facts()?;
                let decl = facts.decl_by_id(&decl)?;
                let value_syntax = decl.value_expr_syntax?;
                let tree = self.syntax_tree()?;
                let node = value_syntax.to_node_from_root(&tree.get_red_root())?;
                let value_expr = LuaExpr::cast(node)?;
                self.require_module_file_of_expr_inner(&value_expr, depth + 1)
            }
            _ => None,
        }
    }

    fn require_module_file_of_call(&self, call: &LuaCallExpr) -> Option<FileId> {
        let prefix = call.get_prefix_expr()?;
        let LuaExpr::NameExpr(name_expr) = prefix else {
            return None;
        };
        if name_expr.get_name_text().as_deref() != Some("require") {
            return None;
        }
        let arg = call.get_args_list()?.get_args().next()?;
        let module_name = match self.type_of_expr(arg.get_syntax_id()) {
            LuaType::StringConst(s) | LuaType::DocStringConst(s) => s.as_ref().to_string(),
            _ => return None,
        };
        self.module_file_of(&module_name)
    }

    fn module_export_owner_of(&self, module_file: FileId) -> Option<SemanticId> {
        let module_facts = self.file_facts_of(module_file)?;
        match &module_facts.module_export {
            ModuleExport::Decl { decl, .. } => Some(decl.clone()),
            ModuleExport::Expr { value_syntax } => {
                Some(SemanticId::member(module_file, value_syntax.get_range()))
            }
            _ => None,
        }
    }
}

/// The keys of the name chain a key is accessed through, root excluded (`a.b.c` -> `[b]`).
fn index_prefix_path(index_expr: &LuaIndexExpr) -> Option<Vec<SmolStr>> {
    let mut path = Vec::new();
    let mut current = index_expr.get_prefix_expr()?;
    loop {
        match current {
            LuaExpr::ParenExpr(paren) => current = paren.get_expr()?,
            LuaExpr::IndexExpr(parent) => {
                path.push(SmolStr::new(parent.get_index_key()?.get_path_part()));
                current = parent.get_prefix_expr()?;
            }
            LuaExpr::NameExpr(_) => break,
            _ => return None,
        }
    }
    path.reverse();
    Some(path)
}
