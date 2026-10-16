#[cfg(test)]
mod tests {
    use crate::handlers::test_lib::{ProviderVirtualWorkspace, VirtualLocation, check};
    use googletest::prelude::*;

    fn at(file: &str, line: u32) -> VirtualLocation {
        VirtualLocation {
            file: file.to_string(),
            line,
        }
    }

    fn workspace_with_outer(outer: &str) -> ProviderVirtualWorkspace {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "inner.lua",
            r#"
                local M = {}
                function M.root_pattern() end
                return M
            "#,
        );
        ws.def_file("outer.lua", outer);
        ws
    }

    #[gtest]
    fn test_definition_through_require_held_in_table_constructor() -> Result<()> {
        let mut ws = workspace_with_outer(
            r#"
                local M = {
                    util = require 'inner';
                }
                return M
            "#,
        );
        check!(ws.check_definition(
            r#"
                local outer = require('outer')
                outer.util.root_<??>pattern()
            "#,
            vec![at("inner.lua", 2)]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_through_require_assigned_to_a_field() -> Result<()> {
        let mut ws = workspace_with_outer(
            r#"
                local M = {}
                M.util = require('inner')
                return M
            "#,
        );
        check!(ws.check_definition(
            r#"
                local outer = require('outer')
                outer.util.root_<??>pattern()
            "#,
            vec![at("inner.lua", 2)]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_through_module_returned_by_setmetatable() -> Result<()> {
        let mut ws = workspace_with_outer(
            r#"
                local M = {
                    util = require 'inner';
                }
                return setmetatable(M, { __index = function() end })
            "#,
        );
        check!(ws.check_definition(
            r#"
                local outer = require('outer')
                outer.util.root_<??>pattern()
            "#,
            vec![at("inner.lua", 2)]
        ));
        Ok(())
    }
}
