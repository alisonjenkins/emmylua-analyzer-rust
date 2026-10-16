#[cfg(test)]
mod tests {
    use crate::handlers::test_lib::{ProviderVirtualWorkspace, VirtualLocation, check};
    use googletest::prelude::*;

    type Expected = VirtualLocation;

    fn at(file: &str, line: u32) -> Expected {
        Expected {
            file: file.to_string(),
            line,
        }
    }

    const MODULE_WITH_EXPORTED_LOCAL: &str = r#"
        local _M = {}
        local function deepcopy(x) return x end
        _M.deepcopy = deepcopy
        return _M
    "#;

    #[gtest]
    fn test_definition_of_alias_field_lists_local_function_and_assignment() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("util.lua", MODULE_WITH_EXPORTED_LOCAL);
        check!(ws.check_definition(
            r#"
                local util = require('util')
                util.deep<??>copy(1)
            "#,
            vec![at("util.lua", 2), at("util.lua", 3)]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_inline_require_field_lists_local_function_and_assignment() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("util.lua", MODULE_WITH_EXPORTED_LOCAL);
        check!(ws.check_definition(
            r#"
                require('util').deep<??>copy(1)
            "#,
            vec![at("util.lua", 2), at("util.lua", 3)]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_local_alias_of_global_path_field_reaches_origin() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "lib.lua",
            r#"
                lib = {}
                lib.sub = {}
                function lib.sub._getline() end
            "#,
        );
        check!(ws.check_definition(
            r#"
                local getline = lib.sub._getline
                get<??>line()
            "#,
            vec![at("lib.lua", 3), at("", 1)]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_table_key_with_call_value_reaches_callee() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local M = {}
                function M.make(opts) return function() end end
                local picker = {
                    entry_ma<??>ker = M.make(1),
                }
            "#,
            vec![at("", 2), at("", 4)]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_local_assigned_call_reaches_returned_closure() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local function make() return function() end end
                local han<??>dler = make()
            "#,
            vec![at("", 1), at("", 2)]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_field_assigned_call_reaches_callee_and_returned_closure() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local util = {}
                function util.add_hook(f)
                    return function() end
                end
                local cfg = {}
                cfg.on_i<??>nit = util.add_hook(1)
            "#,
            vec![at("", 3), at("", 6)]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_local_assigned_call_with_declared_return_skips_the_body() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                ---@return function
                local function make() return function() end end
                local han<??>dler = make()
            "#,
            vec![at("", 3)]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_field_assigned_parameter_stays_on_the_field() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local function build(name)
                    return { na<??>me = name }
                end
            "#,
            vec![at("", 2)]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_field_assigned_local_function_reaches_function()-> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local M = {}
                local function impl() end
                M.pu<??>blic = impl
            "#,
            vec![at("", 2), at("", 3)]
        ));
        Ok(())
    }
}
