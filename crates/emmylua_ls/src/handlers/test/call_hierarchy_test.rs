#[cfg(test)]
mod tests {
    use crate::handlers::call_hierarchy::{incoming_calls, outgoing_calls, prepare_call_hierarchy};
    use crate::handlers::test_lib::{ProviderVirtualWorkspace, check};
    use googletest::prelude::*;

    /// `(caller name, call-site lines)` for every incoming call to the function under `<??>`.
    fn incoming(ws: &mut ProviderVirtualWorkspace, block: &str) -> Result<Vec<(String, Vec<u32>)>> {
        let (content, position) = check!(ProviderVirtualWorkspace::handle_file_content(block));
        let file_id = ws.def(&content);
        incoming_at(ws, file_id, position)
    }

    /// Like `incoming`, with the function declared in `mod.lua` and used by `consumer.lua`, which
    /// is loaded after it.
    fn incoming_from_module(
        ws: &mut ProviderVirtualWorkspace,
        module_block: &str,
        consumer: &str,
    ) -> Result<Vec<(String, Vec<u32>)>> {
        let (content, position) = check!(ProviderVirtualWorkspace::handle_file_content(
            module_block
        ));
        let file_id = ws.def_file("mod.lua", &content);
        ws.def(consumer);
        incoming_at(ws, file_id, position)
    }

    fn incoming_at(
        ws: &mut ProviderVirtualWorkspace,
        file_id: emmylua_code_analysis::FileId,
        position: lsp_types::Position,
    ) -> Result<Vec<(String, Vec<u32>)>> {
        let items = prepare_call_hierarchy(&ws.analysis, file_id, position)
            .ok_or("prepare failed")
            .or_fail()?;
        let calls = incoming_calls(&ws.analysis, &items[0])
            .ok_or("incoming failed")
            .or_fail()?;
        let mut out: Vec<(String, Vec<u32>)> = calls
            .into_iter()
            .map(|call| {
                let mut lines: Vec<u32> = call.from_ranges.iter().map(|r| r.start.line).collect();
                lines.sort();
                (call.from.name, lines)
            })
            .collect();
        out.sort();
        Ok(out)
    }

    #[gtest]
    fn test_incoming_ignores_declaration_and_non_call_references() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(incoming(
            &mut ws,
            r#"
                local function t<??>arget() end
                local function caller()
                    target()
                end
                local alias = target
                local t = { cb = target }
            "#
        ));
        verify_eq!(got, vec![("caller".to_string(), vec![3])])
    }

    #[gtest]
    fn test_incoming_groups_call_sites_per_caller() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(incoming(
            &mut ws,
            r#"
                local function t<??>arget() end
                local function caller()
                    target()
                    target()
                end
            "#
        ));
        verify_eq!(got, vec![("caller".to_string(), vec![3, 4])])
    }

    #[gtest]
    fn test_incoming_from_method_and_field_functions() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(incoming(
            &mut ws,
            r#"
                local M = {}
                function M.t<??>arget() end
                function M.a()
                    M.target()
                end
                function M:b()
                    M.target()
                end
                M.c = function()
                    M.target()
                end
            "#
        ));
        verify_eq!(
            got,
            vec![
                ("M.a".to_string(), vec![4]),
                ("M.b".to_string(), vec![7]),
                ("M.c".to_string(), vec![10]),
            ]
        )
    }

    #[gtest]
    fn test_incoming_ignores_calls_of_fields_set_to_the_result_of_a_call() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(incoming(
            &mut ws,
            r#"
                local function make<??>_logger(level)
                    return function() end
                end
                local log = {}
                log.debug = make_logger('DEBUG')
                local function caller()
                    log.debug()
                end
            "#
        ));
        verify_eq!(got, vec![("virtual_0.lua".to_string(), vec![5])])
    }

    #[gtest]
    fn test_incoming_caller_item_resolves_to_its_own_function() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let (content, position) = check!(ProviderVirtualWorkspace::handle_file_content(
            r#"
                local M = {}
                function M.t<??>arget() end
                function M.mid()
                    M.target()
                end
                function M.top()
                    M.mid()
                end
            "#
        ));
        let file_id = ws.def(&content);
        let items = prepare_call_hierarchy(&ws.analysis, file_id, position).or_fail()?;
        let calls = incoming_calls(&ws.analysis, &items[0]).or_fail()?;
        verify_eq!(calls.len(), 1)?;
        let up = incoming_calls(&ws.analysis, &calls[0].from).or_fail()?;
        let names: Vec<_> = up.iter().map(|c| c.from.name.clone()).collect();
        verify_eq!(names, vec!["M.top".to_string()])
    }

    #[gtest]
    fn test_incoming_includes_calls_on_receivers_known_by_type() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "a.lua",
            r#"
                local reader = require("virtual_0")
                local function use_it()
                    local r = reader.open("x")
                    r:close()
                end
            "#,
        );
        let got = check!(incoming(
            &mut ws,
            r#"
                ---@class Reader
                local M = {}
                M.__index = M

                ---@return Reader
                function M.open(path)
                    return setmetatable({ path = path }, M)
                end

                function M:cl<??>ose()
                end

                return M
            "#
        ));
        verify_eq!(got, vec![("use_it".to_string(), vec![4])])
    }

    #[gtest]
    fn test_incoming_calls_on_the_result_of_calling_a_required_function() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(incoming_from_module(
            &mut ws,
            r#"
                return function(ctx)
                    local e = {}
                    function e.s<??>et(k) end
                    return e
                end
            "#,
            r#"
                local make = require("mod")
                local env = make(1)
                local function use_it()
                    env.set(1)
                end
            "#,
        ));
        verify_eq!(got, vec![("use_it".to_string(), vec![4])])
    }

    #[gtest]
    fn test_incoming_calls_on_the_result_of_calling_a_require_expression() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(incoming_from_module(
            &mut ws,
            r#"
                return function(ctx)
                    local e = {}
                    function e.s<??>et(k) end
                    return e
                end
            "#,
            r#"
                local env = require("mod")(1)
                local function use_it()
                    env.set(1)
                end
            "#,
        ));
        verify_eq!(got, vec![("use_it".to_string(), vec![3])])
    }

    #[gtest]
    fn test_incoming_top_level_calls_are_grouped_into_one_module_caller() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(incoming(
            &mut ws,
            r#"
                local function t<??>arget() end
                target()
                target()
            "#
        ));
        verify_eq!(got, vec![("virtual_0.lua".to_string(), vec![2, 3])])
    }

    /// `(callee name, call-site lines)` for every outgoing call of the function under `<??>`.
    fn outgoing(ws: &mut ProviderVirtualWorkspace, block: &str) -> Result<Vec<(String, Vec<u32>)>> {
        let (content, position) = check!(ProviderVirtualWorkspace::handle_file_content(block));
        let file_id = ws.def(&content);
        let items = prepare_call_hierarchy(&ws.analysis, file_id, position)
            .ok_or("prepare failed")
            .or_fail()?;
        let calls = outgoing_calls(&ws.analysis, &items[0])
            .ok_or("outgoing failed")
            .or_fail()?;
        Ok(calls
            .into_iter()
            .map(|call| {
                let mut lines: Vec<u32> = call.from_ranges.iter().map(|r| r.start.line).collect();
                lines.sort();
                (call.to.name, lines)
            })
            .collect())
    }

    #[gtest]
    fn test_outgoing_lists_callees_grouped_with_call_sites() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(outgoing(
            &mut ws,
            r#"
                local function helper() end
                local M = {}
                function M.util() end
                function M.r<??>un()
                    helper()
                    helper()
                    M.util()
                end
            "#
        ));
        verify_eq!(
            got,
            vec![
                ("helper".to_string(), vec![5, 6]),
                ("util".to_string(), vec![7]),
            ]
        )
    }

    #[gtest]
    fn test_outgoing_includes_calls_in_callbacks_and_nested_functions() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(outgoing(
            &mut ws,
            r#"
                local function each(cb) end
                local function inner_only() end
                local function in_callback() end
                local M = {}
                function M:oth() end
                function M:r<??>un()
                    self:oth()
                    each(function()
                        in_callback()
                    end)
                    local function nested()
                        inner_only()
                    end
                end
            "#
        ));
        verify_eq!(
            got,
            vec![
                ("oth".to_string(), vec![7]),
                ("each".to_string(), vec![8]),
                ("in_callback".to_string(), vec![9]),
                ("inner_only".to_string(), vec![12]),
            ]
        )
    }

    #[gtest]
    fn test_outgoing_lists_function_a_field_is_set_to() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(outgoing(
            &mut ws,
            r#"
                local function impl() end
                local M = { work = impl }
                local function r<??>un()
                    M.work()
                end
            "#
        ));
        let mut names: Vec<String> = got.iter().map(|(name, _)| name.clone()).collect();
        names.sort();
        verify_eq!(names, vec!["impl".to_string(), "work".to_string()])
    }

    #[gtest]
    fn test_outgoing_lists_every_file_defining_a_global_function() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("a.lua", "function shared() end");
        ws.def_file("b.lua", "function shared() end");
        let (content, position) = check!(ProviderVirtualWorkspace::handle_file_content(
            r#"
                local function r<??>un()
                    shared()
                end
            "#
        ));
        let file_id = ws.def(&content);
        let items = prepare_call_hierarchy(&ws.analysis, file_id, position)
            .ok_or("prepare failed")
            .or_fail()?;
        let calls = outgoing_calls(&ws.analysis, &items[0])
            .ok_or("outgoing failed")
            .or_fail()?;
        let mut uris: Vec<String> = calls
            .iter()
            .map(|call| {
                let path = call.to.uri.path();
                path.rsplit('/').next().unwrap_or(path).to_string()
            })
            .collect();
        uris.sort();
        verify_eq!(uris, vec!["a.lua".to_string(), "b.lua".to_string()])
    }

    #[gtest]
    fn test_outgoing_of_closure_assigned_to_a_field() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let got = check!(outgoing(
            &mut ws,
            r#"
                local function helper() end
                local M = {}
                M.r<??>un = function()
                    helper()
                end
            "#
        ));
        verify_eq!(got, vec![("helper".to_string(), vec![4])])
    }

    #[gtest]
    fn test_outgoing_follows_cross_file_require() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "dep.lua",
            r#"
                local M = {}
                function M.work() end
                return M
            "#,
        );
        let got = check!(outgoing(
            &mut ws,
            r#"
                local dep = require("dep")
                local function r<??>un()
                    dep.work()
                end
            "#
        ));
        verify_eq!(got, vec![("work".to_string(), vec![3])])
    }
}
