#[cfg(test)]
mod tests {
    use googletest::prelude::*;
    use lsp_types::{SymbolKind, WorkspaceSymbol, WorkspaceSymbolResponse};
    use tokio_util::sync::CancellationToken;

    use crate::handlers::test_lib::ProviderVirtualWorkspace;
    use crate::handlers::workspace_symbol::build_workspace_symbols;

    const FIXTURE: &str = r#"
        local M = {}

        function M.parse_line(line)
        end

        function M:method()
        end

        M.slugify = function()
        end

        M.count = 1

        ---@class Foo
        ---@field bar integer

        GlobalThing = 1

        return M
    "#;

    fn query(ws: &ProviderVirtualWorkspace, query: &str) -> Vec<WorkspaceSymbol> {
        match build_workspace_symbols(&ws.analysis, query.to_string(), CancellationToken::new()) {
            Some(WorkspaceSymbolResponse::Nested(symbols)) => symbols,
            _ => Vec::new(),
        }
    }

    fn kind_of(symbols: &[WorkspaceSymbol], name: &str) -> Option<SymbolKind> {
        let mut matching = symbols.iter().filter(|symbol| symbol.name == name);
        let first = matching.next()?;
        // one declaration must yield one symbol
        if matching.next().is_some() {
            return None;
        }
        Some(first.kind)
    }

    #[gtest]
    fn test_function_member_is_found() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(FIXTURE);
        let symbols = query(&ws, "parse_line");
        expect_that!(
            kind_of(&symbols, "parse_line"),
            some(eq(SymbolKind::FUNCTION))
        );
        Ok(())
    }

    #[gtest]
    fn test_method_member_is_found() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(FIXTURE);
        let symbols = query(&ws, "method");
        expect_that!(kind_of(&symbols, "method"), some(eq(SymbolKind::METHOD)));
        Ok(())
    }

    #[gtest]
    fn test_function_valued_field_is_found() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(FIXTURE);
        let symbols = query(&ws, "slugify");
        expect_that!(kind_of(&symbols, "slugify"), some(eq(SymbolKind::FUNCTION)));
        Ok(())
    }

    #[gtest]
    fn test_plain_field_is_found() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(FIXTURE);
        expect_that!(
            kind_of(&query(&ws, "count"), "count"),
            some(eq(SymbolKind::FIELD))
        );
        Ok(())
    }

    #[gtest]
    fn test_doc_field_is_found_with_class_container() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(FIXTURE);
        let symbols = query(&ws, "bar");
        expect_that!(kind_of(&symbols, "bar"), some(eq(SymbolKind::FIELD)));
        let container = symbols
            .iter()
            .find(|symbol| symbol.name == "bar")
            .and_then(|symbol| symbol.container_name.as_deref());
        expect_that!(container, some(eq("Foo")));
        Ok(())
    }

    #[gtest]
    fn test_unmatched_query_returns_nothing() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(FIXTURE);
        expect_that!(query(&ws, "zzz_no_such_symbol"), is_empty());
        Ok(())
    }

    #[gtest]
    fn test_globals_and_classes_still_found() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(FIXTURE);
        expect_that!(
            query(&ws, "GlobalThing")
                .iter()
                .any(|symbol| symbol.name == "GlobalThing"),
            eq(true)
        );
        expect_that!(
            kind_of(&query(&ws, "Foo"), "Foo"),
            some(eq(SymbolKind::CLASS))
        );
        Ok(())
    }
}
