mod node;
mod traits;
mod tree;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::iter::successors;
use std::marker::PhantomData;

use rowan::{Language, TextRange, TextSize};

use crate::kind::{LuaKind, LuaSyntaxKind, LuaTokenKind};
pub use traits::*;
pub use tree::{LuaSyntaxTree, LuaTreeBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LuaLanguage;

impl Language for LuaLanguage {
    type Kind = LuaKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        LuaKind::from_raw(raw.0)
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind.get_raw())
    }
}

pub type LuaSyntaxNode = rowan::SyntaxNode<LuaLanguage>;
pub type LuaSyntaxToken = rowan::SyntaxToken<LuaLanguage>;
pub type LuaSyntaxElement = rowan::NodeOrToken<LuaSyntaxNode, LuaSyntaxToken>;
pub type LuaSyntaxElementChildren = rowan::SyntaxElementChildren<LuaLanguage>;
pub type LuaSyntaxNodeChildren = rowan::SyntaxNodeChildren<LuaLanguage>;
pub type LuaSyntaxNodePtr = rowan::ast::SyntaxNodePtr<LuaLanguage>;

impl From<LuaSyntaxKind> for rowan::SyntaxKind {
    fn from(kind: LuaSyntaxKind) -> Self {
        let lua_kind = LuaKind::from(kind);
        rowan::SyntaxKind(lua_kind.get_raw())
    }
}

impl From<rowan::SyntaxKind> for LuaSyntaxKind {
    fn from(kind: rowan::SyntaxKind) -> Self {
        LuaKind::from_raw(kind.0).into()
    }
}

impl From<LuaTokenKind> for rowan::SyntaxKind {
    fn from(kind: LuaTokenKind) -> Self {
        let lua_kind = LuaKind::from(kind);
        rowan::SyntaxKind(lua_kind.get_raw())
    }
}

impl From<rowan::SyntaxKind> for LuaTokenKind {
    fn from(kind: rowan::SyntaxKind) -> Self {
        LuaKind::from_raw(kind.0).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LuaSyntaxId {
    kind: LuaKind,
    range: TextRange,
}

impl LuaSyntaxId {
    pub fn new(kind: LuaKind, range: TextRange) -> Self {
        LuaSyntaxId { kind, range }
    }

    pub fn from_ptr(ptr: LuaSyntaxNodePtr) -> Self {
        LuaSyntaxId {
            kind: ptr.kind(),
            range: ptr.text_range(),
        }
    }

    pub fn from_node(node: &LuaSyntaxNode) -> Self {
        LuaSyntaxId {
            kind: node.kind(),
            range: node.text_range(),
        }
    }

    pub fn from_token(token: &LuaSyntaxToken) -> Self {
        LuaSyntaxId {
            kind: token.kind(),
            range: token.text_range(),
        }
    }

    pub fn get_kind(&self) -> LuaSyntaxKind {
        self.kind.into()
    }

    pub fn get_token_kind(&self) -> LuaTokenKind {
        self.kind.into()
    }

    pub fn is_token(&self) -> bool {
        self.kind.is_token()
    }

    pub fn is_node(&self) -> bool {
        self.kind.is_syntax()
    }

    pub fn get_range(&self) -> TextRange {
        self.range
    }

    pub fn to_node(&self, tree: &LuaSyntaxTree) -> Option<LuaSyntaxNode> {
        let root = tree.get_red_root();
        if root.parent().is_some() {
            return None;
        }
        self.to_node_from_root(&root)
    }

    pub fn to_node_from_root(&self, root: &LuaSyntaxNode) -> Option<LuaSyntaxNode> {
        Self::find_node_at_range(root, self.range, |kind| kind == self.kind)
    }

    /// First node on the descent path to `range` whose range is exactly `range` and whose kind
    /// satisfies `accept`. Several nodes can share a range (`{}` and its field value wrapper).
    pub fn find_node_at_range(
        root: &LuaSyntaxNode,
        range: TextRange,
        accept: impl Fn(LuaKind) -> bool,
    ) -> Option<LuaSyntaxNode> {
        successors(Some(root.clone()), |node| {
            node.child_or_token_at_range(range)?.into_node()
        })
        .find(|it| it.text_range() == range && accept(it.kind()))
    }

    pub fn to_token(&self, tree: &LuaSyntaxTree) -> Option<LuaSyntaxToken> {
        let root = tree.get_red_root();
        if root.parent().is_some() {
            return None;
        }
        self.to_token_from_root(&root)
    }

    pub fn to_token_from_root(&self, root: &LuaSyntaxNode) -> Option<LuaSyntaxToken> {
        let mut current_node = Some(root.clone());
        while let Some(node) = current_node {
            let node_or_token = node.child_or_token_at_range(self.range)?;
            match node_or_token {
                rowan::NodeOrToken::Node(node) => {
                    current_node = Some(node);
                }
                rowan::NodeOrToken::Token(token) => {
                    if token.text_range() == self.range && token.kind() == self.kind {
                        return Some(token);
                    }
                    return None;
                }
            }
        }
        None
    }

    pub fn to_node_at_range(root: &LuaSyntaxNode, range: TextRange) -> Option<LuaSyntaxNode> {
        successors(Some(root.clone()), |node| {
            node.child_or_token_at_range(range)?.into_node()
        })
        .find(|it| it.text_range() == range)
    }
}

impl Serialize for LuaSyntaxId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let kind_raw = self.kind.get_raw();
        let start = u32::from(self.range.start());
        let end = u32::from(self.range.end());
        let range_combined = ((start as u64) << 32) | (end as u64);
        let value = format!("{:x}:{:x}", kind_raw, range_combined);
        serializer.serialize_str(&value)
    }
}

impl<'de> Deserialize<'de> for LuaSyntaxId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LuaSyntaxIdVisitor;

        impl Visitor<'_> for LuaSyntaxIdVisitor {
            type Value = LuaSyntaxId;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string with format 'kind:range'")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let parts: Vec<&str> = value.split(':').collect();
                if parts.len() != 2 {
                    return Err(E::custom("expected format 'kind:range'"));
                }

                let kind_raw = u16::from_str_radix(parts[0], 16)
                    .map_err(|e| E::custom(format!("invalid kind: {}", e)))?;
                let range_combined = u64::from_str_radix(parts[1], 16)
                    .map_err(|e| E::custom(format!("invalid range: {}", e)))?;

                let start = TextSize::new(((range_combined >> 32) & 0xFFFFFFFF) as u32);
                let end = TextSize::new((range_combined & 0xFFFFFFFF) as u32);

                Ok(LuaSyntaxId {
                    kind: LuaKind::from_raw(kind_raw),
                    range: TextRange::new(start, end),
                })
            }
        }

        deserializer.deserialize_str(LuaSyntaxIdVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LuaAstPtr<T: LuaAstNode> {
    pub syntax_id: LuaSyntaxId,
    _phantom: PhantomData<T>,
}

impl<T: LuaAstNode> LuaAstPtr<T> {
    pub fn new(node: &T) -> Self {
        LuaAstPtr {
            syntax_id: node.get_syntax_id(),
            _phantom: PhantomData,
        }
    }

    pub fn get_syntax_id(&self) -> LuaSyntaxId {
        self.syntax_id
    }

    pub fn to_node(&self, root: &LuaChunk) -> Option<T> {
        let syntax_node = self.syntax_id.to_node_from_root(root.syntax());
        if let Some(node) = syntax_node {
            T::cast(node)
        } else {
            None
        }
    }
}

unsafe impl<T: LuaAstNode> Send for LuaAstPtr<T> {}
unsafe impl<T: LuaAstNode> Sync for LuaAstPtr<T> {}

#[cfg(test)]
mod syntax_id_lookup_tests {
    use super::*;
    use crate::{LuaParser, ParserConfig};

    const FIXTURE: &str = r#"
---@class Point
---@field x number
local M = {}

local defaults = { a = { b = { c = {} } }, list = { 1, 2, { 3 } }, [1] = {}, f = function() return {} end }

function M.new(x, y)
    local t = { x = x, y = y, nested = { inner = { x, y } } }
    return setmetatable(t, { __index = M })
end

function M:method(...)
    for i, v in ipairs({ ... }) do
        if v then
            print(("%d"):format(i), M.new(v, v).x, defaults.a.b.c)
        end
    end
    return (function() return { self } end)()
end

return M
"#;

    // The implementation this module replaced, kept as the oracle.
    fn legacy_to_node_from_root(id: LuaSyntaxId, root: &LuaSyntaxNode) -> Option<LuaSyntaxNode> {
        successors(Some(root.clone()), |node| {
            node.child_or_token_at_range(id.range)?.into_node()
        })
        .find(|it| it.text_range() == id.range && it.kind() == id.kind)
    }

    fn ids_to_probe(root: &LuaSyntaxNode) -> Vec<LuaSyntaxId> {
        let mut ids = Vec::new();
        for node in root.descendants() {
            let id = LuaSyntaxId::from_node(&node);
            ids.push(id);
            // Same range, other kind: must not resolve to a node of the wrong kind.
            ids.push(LuaSyntaxId::new(
                LuaKind::from(LuaSyntaxKind::Chunk),
                id.range,
            ));
            ids.push(LuaSyntaxId::new(
                LuaKind::from(LuaSyntaxKind::TableEmptyExpr),
                id.range,
            ));
            // Same kind, longer range: no node has it.
            let longer = TextRange::new(id.range.start(), id.range.end() + TextSize::new(1));
            ids.push(LuaSyntaxId::new(id.kind, longer));
        }
        for token in root.descendants_with_tokens().filter_map(|e| e.into_token()) {
            ids.push(LuaSyntaxId::from_token(&token));
        }
        ids
    }

    #[test]
    fn to_node_from_root_matches_legacy_for_every_id() {
        let tree = LuaParser::parse(FIXTURE, ParserConfig::default());
        let root = tree.get_red_root();
        let ids = ids_to_probe(&root);
        assert!(ids.len() > 400, "fixture too small: {}", ids.len());
        let mut resolved = 0;
        for id in ids {
            let new = id.to_node_from_root(&root);
            assert_eq!(new, legacy_to_node_from_root(id, &root), "{id:?}");
            if let Some(node) = new {
                assert_eq!(LuaSyntaxId::from_node(&node), id);
                resolved += 1;
            }
        }
        assert!(resolved > 100, "too few ids resolved: {resolved}");
    }

    #[test]
    fn find_node_at_range_selects_by_kind_among_same_range_nodes() {
        let tree = LuaParser::parse("local t = { { } }", ParserConfig::default());
        let root = tree.get_red_root();
        let inner = root
            .descendants()
            .find(|n| n.kind() == LuaKind::from(LuaSyntaxKind::TableEmptyExpr))
            .unwrap();
        // The table field value wraps the empty table with an identical range.
        let wrapper =
            LuaSyntaxId::find_node_at_range(&root, inner.text_range(), |_| true).unwrap();
        assert_ne!(wrapper.kind(), inner.kind());
        let found = LuaSyntaxId::find_node_at_range(&root, inner.text_range(), |k| {
            k == LuaKind::from(LuaSyntaxKind::TableEmptyExpr)
        });
        assert_eq!(found, Some(inner));
    }
}
