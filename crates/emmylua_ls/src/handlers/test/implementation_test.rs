#[cfg(test)]
mod tests {
    use crate::handlers::test_lib::{ProviderVirtualWorkspace, VirtualLocation, check};
    use googletest::prelude::*;

    #[gtest]
    fn test_cursor_at_start_of_identifier() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_implementation(
            r#"
                local function <??>save()
                end
                save()
            "#,
            vec![VirtualLocation {
                file: "".to_string(),
                line: 1,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_1() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "2.lua",
            r#"
               delete = require("virtual_0").delete
               delete()
            "#,
        );
        ws.def_file(
            "3.lua",
            r#"
               delete = require("virtual_0").delete
               delete()
            "#,
        );
        check!(ws.check_implementation(
            r#"
                local M = {}
                function M.de<??>lete(a)
                end
                return M
            "#,
            vec![VirtualLocation {
                file: "".to_string(),
                line: 2,
            }],
        ));
        Ok(())
    }

    // M3 batch 3 retirement: origin-owner penetration / partial class merging / full collection of same-named global declarations depend on the old DbIndex; see migration notes §M3.
    #[gtest]
    fn test_2() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "1.lua",
            r#"
                ---@class (partial) Test
                test = {}

                test.a = 1
            "#,
        );
        ws.def_file(
            "2.lua",
            r#"
                ---@class (partial) Test
                test = {}
                test.a = 1
            "#,
        );
        ws.def_file(
            "3.lua",
            r#"
                local a = test.a
            "#,
        );
        check!(ws.check_implementation(
            r#"
                t<??>est
            "#,
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "2.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 2,
                }
            ],
        ));
        Ok(())
    }

    // M3 batch 3 retirement: origin-owner penetration / partial class merging / full collection of same-named global declarations depend on the old DbIndex; see migration notes §M3.
    #[gtest]
    fn test_3() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "1.lua",
            r#"
                ---@class YYY
                ---@field a number
                yyy = {}

                if false then
                    yyy.a = 1
                    if yyy.a then
                    end
                end
            "#,
        );
        check!(ws.check_implementation(
            r#"
                yyy.<??>a = 2
            "#,
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 6,
                },
            ],
        ));
        Ok(())
    }

    // M3 batch 3 retirement: origin-owner penetration / partial class merging / full collection of same-named global declarations depend on the old DbIndex; see migration notes §M3.
    #[gtest]
    fn test_table_field_definition_1() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_implementation(
            r#"
                ---@class T
                ---@field func fun(self: T) 注释注释

                ---@type T
                local t = {
                    func = function(self)
                    end,
                }

                t:fun<??>c()
            "#,
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 6,
                },
            ],
        ));
        Ok(())
    }

    // M3 batch 3 retirement: origin-owner penetration / partial class merging / full collection of same-named global declarations depend on the old DbIndex; see migration notes §M3.
    #[gtest]
    fn test_table_field_definition_2() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_implementation(
            r#"
                ---@class T
                ---@field func fun(self: T) 注释注释

                ---@type T
                local t = {
                    f<??>unc = function(self)
                    end,
                }
            "#,
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 6,
                },
            ],
        ));
        Ok(())
    }

    // M3 batch 3 retirement: origin-owner penetration / partial class merging / full collection of same-named global declarations depend on the old DbIndex; see migration notes §M3.
    #[gtest]
    fn test_separation_of_define_and_impl() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_implementation(
            r#"
                local a<??>bc

                abc = function()
                end

                local _a = abc
                local _b = abc()

                abc = function()
                end
            "#,
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 3,
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 9,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_same_key_on_other_local_module_is_not_an_implementation() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "other.lua",
            r#"
                local M = {}
                function M.setup(c) end
                return M
            "#,
        );
        check!(ws.check_implementation(
            r#"
                local M = {}
                function M.se<??>tup(c) end
                return M
            "#,
            vec![VirtualLocation {
                file: "".to_string(),
                line: 2,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_same_key_in_other_table_constructor_is_not_an_implementation() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_implementation(
            r#"
                local t = { lab<??>el = 'x', other = 1 }
                local u = { label = 'y' }
                t.label = 'z'
            "#,
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 3,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_same_key_in_sibling_nested_table_is_not_an_implementation() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_implementation(
            r#"
                local cfg = {
                    a = { x<??> = 1 },
                    b = { x = 2 },
                }
            "#,
            vec![VirtualLocation {
                file: "".to_string(),
                line: 2,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_same_method_name_on_other_class_is_not_an_implementation() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "other.lua",
            r#"
                local Cls2 = {}
                function Cls2:new() end
            "#,
        );
        check!(ws.check_implementation(
            r#"
                local Cls = {}
                Cls.__index = Cls
                function Cls:n<??>ew() end
                local o = setmetatable({}, Cls)
                o:new()
            "#,
            vec![VirtualLocation {
                file: "".to_string(),
                line: 3,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_typed_returned_tables_share_their_class_fields() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "a.lua",
            r#"
                ---@class FC
                ---@field meta table
            "#,
        );
        ws.def_file(
            "b.lua",
            r#"
                ---@type FC
                return {
                  meta = {},
                }
            "#,
        );
        check!(ws.check_implementation(
            r#"
                ---@type FC
                return {
                  me<??>ta = {},
                }
            "#,
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 3,
                },
                VirtualLocation {
                    file: "b.lua".to_string(),
                    line: 3,
                },
                VirtualLocation {
                    file: "a.lua".to_string(),
                    line: 2,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_string_literal_table_key() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_implementation(
            r#"
                local steps = {
                    ["unt<??>il"] = function() end,
                }
            "#,
            vec![VirtualLocation {
                file: "".to_string(),
                line: 2,
            }],
        ));
        Ok(())
    }
}
