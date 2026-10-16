#[cfg(test)]
mod tests {
    use googletest::prelude::*;
    use lsp_types::{SymbolKind, SymbolTag, WorkspaceSymbol, WorkspaceSymbolResponse};
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
    fn test_deprecated_member_is_tagged() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
            local M = {}

            ---@deprecated
            function M.old_api() end

            function M.new_api() end

            return M
        "#,
        );
        let tags_of = |name: &str| {
            query(&ws, name)
                .into_iter()
                .find(|symbol| symbol.name == name)
                .map(|symbol| symbol.tags)
        };
        expect_that!(
            tags_of("old_api"),
            some(some(eq(&vec![SymbolTag::DEPRECATED])))
        );
        expect_that!(tags_of("new_api"), some(none()));
        Ok(())
    }

    #[gtest]
    fn test_cancelled_token_returns_no_symbols() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(FIXTURE);
        let cancel = CancellationToken::new();
        cancel.cancel();
        // Cancellation is not an error here: every file bails out early, so
        // the result is an empty Nested response, members and globals alike.
        let response = build_workspace_symbols(&ws.analysis, "".to_string(), cancel);
        expect_that!(
            response,
            some(matches_pattern!(
                WorkspaceSymbolResponse::Nested(is_empty())
            ))
        );
        Ok(())
    }

    // LuaLS 3.18.1 also returns named fields of table constructors
    // (`return { run = function() end }`), so they stay.
    #[gtest]
    fn test_table_constructor_fields_are_found() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
            local t = { ctorfield = 1 }
            local M = {}
            M.modfield = 1
            return { retfn = function() end, M = M, t = t }
        "#,
        );
        expect_that!(
            kind_of(&query(&ws, "retfn"), "retfn"),
            some(eq(SymbolKind::FUNCTION))
        );
        expect_that!(
            kind_of(&query(&ws, "modfield"), "modfield"),
            some(eq(SymbolKind::FIELD))
        );
        Ok(())
    }

    // LuaLS 3.18.1 returns each field of a local constructor once, including
    // nested ones, even when the table is never returned or exported.
    #[gtest]
    fn test_local_constructor_fields_are_found_once() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
            local t = { ctorfield = 1, ctorfn = function() end }
            local u = { inner = { deep = 1 } }
            print(t, u)
        "#,
        );
        for (name, kind) in [
            ("ctorfield", SymbolKind::FIELD),
            ("ctorfn", SymbolKind::FUNCTION),
            ("inner", SymbolKind::FIELD),
            ("deep", SymbolKind::FIELD),
        ] {
            expect_that!(kind_of(&query(&ws, name), name), some(eq(kind)), "{name}");
        }
        let container = query(&ws, "ctorfield")
            .into_iter()
            .find_map(|symbol| symbol.container_name);
        expect_that!(container.as_deref(), some(eq("t")));
        Ok(())
    }

    // LuaLS 3.18.1 returns every local declaration (all with kind Variable and
    // no container): functions, plain locals, multi-name locals, parameters,
    // loop variables and locals in nested scopes, shadowed ones once per
    // declaration.
    const LOCALS_FIXTURE: &str = r#"
        local function lf_top() end
        local lv_num = 1
        local lv_a, lv_b = 1, 2
        local lv_req = require("x")
        local lv_shadow = 1
        local lv_fnexpr = function() end
        local function outer(param_one, param_two)
            local lv_inner = 1
            local lv_shadow = 2
            local function lf_nested() end
            do
                local lv_block = 3
            end
            for loop_i = 1, 3 do end
            for loop_k, loop_v in pairs({}) do end
            return lv_inner
        end
        function lv_obj:method_with_self() end
        return outer
    "#;

    fn count_of(symbols: &[WorkspaceSymbol], name: &str) -> usize {
        symbols.iter().filter(|symbol| symbol.name == name).count()
    }

    #[gtest]
    fn test_local_functions_are_found_as_functions() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(LOCALS_FIXTURE);
        for name in ["lf_top", "lf_nested", "lv_fnexpr", "outer"] {
            expect_that!(
                kind_of(&query(&ws, name), name),
                some(eq(SymbolKind::FUNCTION)),
                "{name}"
            );
        }
        Ok(())
    }

    #[gtest]
    fn test_local_variables_are_found_once_each() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(LOCALS_FIXTURE);
        for name in ["lv_num", "lv_a", "lv_b", "lv_req", "lv_inner", "lv_block"] {
            expect_that!(
                kind_of(&query(&ws, name), name),
                some(eq(SymbolKind::VARIABLE)),
                "{name}"
            );
        }
        Ok(())
    }

    #[gtest]
    fn test_shadowing_locals_are_one_symbol_per_declaration() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(LOCALS_FIXTURE);
        expect_that!(count_of(&query(&ws, "lv_shadow"), "lv_shadow"), eq(2));
        Ok(())
    }

    #[gtest]
    fn test_parameters_and_loop_variables_are_found() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(LOCALS_FIXTURE);
        for name in ["param_one", "param_two", "loop_i", "loop_k", "loop_v"] {
            expect_that!(count_of(&query(&ws, name), name), eq(1), "{name}");
        }
        Ok(())
    }

    #[gtest]
    fn test_implicit_self_and_varargs_are_not_symbols() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
            local M = {}
            function M:with_self(...) end
            return M
        "#,
        );
        let symbols = query(&ws, "");
        expect_that!(count_of(&symbols, "self"), eq(0));
        expect_that!(count_of(&symbols, "..."), eq(0));
        Ok(())
    }

    // LuaLS stops at 1000 results; a one-letter query on a large workspace
    // otherwise returns tens of thousands of symbols.
    #[gtest]
    fn test_results_are_capped_at_one_thousand() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let source: String = (0..1100)
            .map(|i| format!("local cap_{i} = {i}\n"))
            .collect();
        ws.def(&source);
        expect_that!(query(&ws, "cap_").len(), eq(1000));
        Ok(())
    }

    // LuaLS 3.18.1 returns every plain assignment to a name as its own symbol
    // (kind Variable, no container, range on the name token), whether the
    // target is a local, an upvalue, a parameter or a global, at any depth.
    const REASSIGN_FIXTURE: &str = r#"
        local ra_plan = nil
        ra_plan = patch()
        local ra_words
        ra_words = {}
        local ra_inner = 1
        local function ra_fn(ra_param)
            ra_param = 5
            ra_plan = 1
            for ra_i = 1, 3 do
                ra_inner = ra_i
            end
            if cond then
                ra_inner = 3
            end
        end
        RA_GLOBAL = 1
        RA_GLOBAL = 2
        local ra_a, ra_b = 1, 2
        ra_a, ra_b = ra_b, ra_a
        print(ra_plan, ra_words)
    "#;

    fn lines_of(symbols: &[WorkspaceSymbol], name: &str) -> Vec<u32> {
        let mut lines: Vec<u32> = symbols
            .iter()
            .filter(|symbol| symbol.name == name)
            .filter_map(|symbol| match &symbol.location {
                lsp_types::OneOf::Left(location) => Some(location.range.start.line),
                _ => None,
            })
            .collect();
        lines.sort_unstable();
        lines
    }

    #[gtest]
    fn test_top_level_reassignment_is_a_symbol() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(REASSIGN_FIXTURE);
        let symbols = query(&ws, "ra_words");
        expect_that!(lines_of(&symbols, "ra_words"), eq(&vec![3, 4]));
        expect_that!(
            symbols
                .iter()
                .all(|symbol| symbol.kind == SymbolKind::VARIABLE),
            eq(true)
        );
        expect_that!(
            symbols.iter().all(|symbol| symbol.container_name.is_none()),
            eq(true)
        );
        Ok(())
    }

    #[gtest]
    fn test_reassignment_range_is_the_name_token() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def("local ra_tok = 1\nra_tok = 2\n");
        let symbols = query(&ws, "ra_tok");
        let ranges: Vec<(u32, u32, u32)> = symbols
            .iter()
            .filter_map(|symbol| match &symbol.location {
                lsp_types::OneOf::Left(location) => Some((
                    location.range.start.line,
                    location.range.start.character,
                    location.range.end.character,
                )),
                _ => None,
            })
            .collect();
        expect_that!(ranges.contains(&(1, 0, 6)), eq(true), "{ranges:?}");
        Ok(())
    }

    #[gtest]
    fn test_upvalue_and_nested_reassignments_are_symbols() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(REASSIGN_FIXTURE);
        // declaration, top-level assignment, assignment inside the closure
        expect_that!(count_of(&query(&ws, "ra_plan"), "ra_plan"), eq(3));
        // declaration, assignment in a loop, assignment in an if
        expect_that!(count_of(&query(&ws, "ra_inner"), "ra_inner"), eq(3));
        Ok(())
    }

    #[gtest]
    fn test_parameter_reassignment_is_a_symbol() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(REASSIGN_FIXTURE);
        expect_that!(count_of(&query(&ws, "ra_param"), "ra_param"), eq(2));
        Ok(())
    }

    #[gtest]
    fn test_repeated_global_assignment_is_one_symbol_per_site() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(REASSIGN_FIXTURE);
        expect_that!(
            lines_of(&query(&ws, "RA_GLOBAL"), "RA_GLOBAL"),
            eq(&vec![16, 17])
        );
        Ok(())
    }

    #[gtest]
    fn test_multiple_assignment_targets_are_each_a_symbol() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(REASSIGN_FIXTURE);
        // the declaration plus one target in `ra_a, ra_b = ra_b, ra_a`
        expect_that!(count_of(&query(&ws, "ra_a"), "ra_a"), eq(2));
        expect_that!(count_of(&query(&ws, "ra_b"), "ra_b"), eq(2));
        Ok(())
    }

    #[gtest]
    fn test_reads_are_not_symbols() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(REASSIGN_FIXTURE);
        // `print(ra_plan, ra_words)` and the right-hand sides add nothing
        expect_that!(
            lines_of(&query(&ws, "ra_words"), "ra_words"),
            not(contains(eq(&19)))
        );
        expect_that!(count_of(&query(&ws, "cond"), "cond"), eq(0));
        Ok(())
    }

    #[gtest]
    fn test_indexed_assignment_does_not_report_its_base_name() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def("local ra_base = {}\nra_base.x = 1\nra_base[1] = 2\n");
        expect_that!(count_of(&query(&ws, "ra_base"), "ra_base"), eq(1));
        Ok(())
    }

    #[gtest]
    fn test_unreassigned_local_is_still_one_symbol() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def("local ra_once = 1\nprint(ra_once)\nlocal function ra_f() end\nra_f()\n");
        expect_that!(count_of(&query(&ws, "ra_once"), "ra_once"), eq(1));
        expect_that!(count_of(&query(&ws, "ra_f"), "ra_f"), eq(1));
        Ok(())
    }

    #[gtest]
    fn test_reassignment_respects_query_and_cap() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(REASSIGN_FIXTURE);
        expect_that!(count_of(&query(&ws, "RA_GLOBAL"), "ra_plan"), eq(0));
        let source: String = (0..1100).map(|i| format!("capr_{i} = {i}\n")).collect();
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(&source);
        expect_that!(query(&ws, "capr_").len(), eq(1000));
        Ok(())
    }

    // LuaLS lists `t.x = 1; t.x = 2` once per assignment site.
    #[gtest]
    fn test_repeated_field_assignment_is_one_symbol_per_site() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def("local ra_t = {}\nra_t.ra_x = 1\nra_t.ra_x = 2\nreturn ra_t\n");
        expect_that!(lines_of(&query(&ws, "ra_x"), "ra_x"), eq(&vec![1, 2]));
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
