#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::handlers::references::references;
    use crate::handlers::test_lib::{ProviderVirtualWorkspace, VirtualLocation, check};
    use googletest::prelude::*;

    #[gtest]
    fn test_vararg_references_stay_in_the_declaring_function() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(
            ws.check_references(
                r#"
                local function outer(...)
                    print(.<??>..)
                    local function inner(...)
                        print(...)
                    end
                    return ...
                end
            "#,
                vec![],
                vec![1, 2, 6]
                    .into_iter()
                    .map(|line| VirtualLocation {
                        file: "".to_string(),
                        line,
                    })
                    .collect()
            )
        );
        Ok(())
    }

    // M3 batch 2 retirement: module export alias following (`export.flush = flush` → `require("mod").flush`
    // binding chain) requires cross-file alias chain queries at the semantic layer; see migration notes §M3.
    #[gtest]
    fn test_function_references() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
            r#"
                local export = {}
                local function fl<??>ush()
                end
                export.flush = flush
                return export
            "#,
            vec![(
                "1.lua",
                r#"
                    local flush = require("virtual_0").flush
                    flush()
                "#,
            )],
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 4,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 4,
                },
            ]
        ));
        Ok(())
    }

    // M3 batch 2 retirement: module export alias following; see migration notes §M3.
    #[gtest]
    fn test_function_references_2() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
            r#"
                local function fl<??>ush()
                end
                return {
                    flush = flush,
                }
            "#,
            vec![(
                "1.lua",
                r#"
                    local flush = require("virtual_0").flush
                    flush()
                "#,
            )],
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 4,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "1.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 4,
                },
            ]
        ));
        Ok(())
    }

    // M3 batch 2 retirement: `return init` module export → require binding alias following; see migration notes §M3.
    #[gtest]
    fn test_module_return() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();

        check!(ws.check_references(
            r#"
                local function init()
                end
                return in<??>it
            "#,
            vec![(
                "a.lua",
                r#"
                local init = require("virtual_0")
                init()
            "#,
            )],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "a.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "a.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 3,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_module_return_2() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "a.lua",
            r#"
            local function getA()
            end
            return {
                getA = getA
            }
        "#,
        );

        check!(ws.check_references(
            r#"
                local AModule = require("a")
                AMo<??>dule.getA()
            "#,
            vec![],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 2,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_constructor_attribute_references_class_call() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_references(
            r#"
                ---@generic T
                ---@[constructor("init")]
                ---@param class `T`
                ---@return T
                function meta(class)
                    return {}
                end

                ---@class AAAA
                local AAAA = meta("AAAA")

                function AAAA:in<??>it()
                end

                local c = AAAA()
            "#,
            vec![],
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 12,
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 15,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_goto_label_references() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
            r#"
                while true do
                    goto cont
                    ::co<??>nt::
                end
            "#,
            vec![],
            vec![
                VirtualLocation {
                    file: "".to_string(),
                    line: 2,
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
    fn test_references_excludes_declaration_when_requested() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();

        let (main_content, position) = check!(ProviderVirtualWorkspace::handle_file_content(
            r#"
                local tar<??>get = 1
                print(target)
                target = 2
            "#,
        ));
        let file_id = ws.def(&main_content);

        let result = references(&ws.analysis, file_id, position, false)
            .ok_or("failed to get references")
            .or_fail()?;

        let mut lines = result
            .iter()
            .map(|location| location.range.start.line)
            .collect::<Vec<_>>();
        lines.sort();
        verify_eq!(lines, vec![2, 3])
    }

    #[gtest]
    fn test_references_of_string_literal_key_use() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
            r#"
                local t = {
                    ["name"] = 1,
                }
                print(t["na<??>me"], t.name)
                t["name"] = 2
            "#,
            vec![],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 4,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 4,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 5,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_references_of_string_literal_key_in_constructor() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
            r#"
                local t = {
                    ["na<??>me"] = 1,
                }
                print(t["name"], t.name)
            "#,
            vec![],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 4,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 4,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_module_function_references_survive_monkey_patching() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
            r#"
                local M = {}
                function M.open_reader(path)
                end
                local function use()
                    return M.op<??>en_reader("a")
                end
                return M
            "#,
            vec![
                (
                    "a.lua",
                    r#"




                        local sv = require("virtual_0")
                        sv.open_reader("b")
                    "#,
                ),
                (
                    "patch.lua",
                    r#"






                        local sv = require("virtual_0")
                        local real = sv.open_reader
                        sv.open_reader = function() end
                        sv.open_reader = real
                    "#,
                ),
            ],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 5,
                },
                VirtualLocation {
                    file: "a.lua".to_string(),
                    line: 6,
                },
                VirtualLocation {
                    file: "patch.lua".to_string(),
                    line: 8,
                },
                VirtualLocation {
                    file: "patch.lua".to_string(),
                    line: 9,
                },
                VirtualLocation {
                    file: "patch.lua".to_string(),
                    line: 10,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_references_of_field_on_inline_require() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "mod_a.lua",
            r#"
                local M = {}
                function M.translate(s) return s end
                return M
            "#,
        );
        check!(ws.check_references(
            r#"
                local tr = require("mod_a").trans<??>late
            "#,
            vec![(
                "b.lua",
                r#"
                    local x = require("mod_a")
                    x.translate("s")
                "#,
            )],
            vec![
                VirtualLocation {
                    file: "mod_a.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "b.lua".to_string(),
                    line: 2,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_method_references_through_inferred_receiver_types() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
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
            "#,
            vec![(
                "a.lua",
                r#"
                    local reader = require("virtual_0")
                    local r = reader.open("x")
                    r:close()
                "#,
            )],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 10,
                },
                VirtualLocation {
                    file: "a.lua".to_string(),
                    line: 3,
                },
            ],
        ));
        Ok(())
    }

    fn check_unannotated_constructor_method_references(
        module: &str,
        close_line: u32,
    ) -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
            module,
            vec![(
                "a.lua",
                r#"
                    local reader = require("virtual_0")
                    local r = reader.open("x")
                    r:close()
                "#,
            )],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: close_line,
                },
                VirtualLocation {
                    file: "a.lua".to_string(),
                    line: 3,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_method_references_through_unannotated_setmetatable_return() -> Result<()> {
        check_unannotated_constructor_method_references(
            r#"
                ---@class Reader
                local M = {}
                M.__index = M

                function M.open(path)
                    return setmetatable({ path = path }, M)
                end

                function M:cl<??>ose()
                end

                return M
            "#,
            9,
        )
    }

    #[gtest]
    fn test_method_references_through_unannotated_constructor_with_error_return() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_references(
            r#"
                local M = {}
                local Reader = {}
                Reader.__index = Reader

                function M.open(path)
                    if not path then return nil, "no path" end
                    return setmetatable({ path = path }, Reader)
                end

                function Reader:cl<??>ose()
                end

                return M
            "#,
            vec![(
                "a.lua",
                r#"
                    local reader = require("virtual_0")
                    local r = assert(reader.open("x"))
                    r:close()
                    local q, err = reader.open("y")
                    q:close()
                "#,
            )],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 10,
                },
                VirtualLocation {
                    file: "a.lua".to_string(),
                    line: 3,
                },
                VirtualLocation {
                    file: "a.lua".to_string(),
                    line: 5,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_method_references_through_unannotated_class_without_doc_class() -> Result<()> {
        check_unannotated_constructor_method_references(
            r#"
                local M = {}
                M.__index = M

                function M.open(path)
                    return setmetatable({ path = path }, M)
                end

                function M:cl<??>ose()
                end

                return M
            "#,
            8,
        )
    }

    #[gtest]
    fn test_method_references_through_unannotated_self_local_return() -> Result<()> {
        check_unannotated_constructor_method_references(
            r#"
                local M = {}
                M.__index = M

                function M.open(path)
                    local self = setmetatable({}, M)
                    self.path = path
                    return self
                end

                function M:cl<??>ose()
                end

                return M
            "#,
            10,
        )
    }

    #[gtest]
    fn test_method_references_through_unannotated_inline_index_metatable() -> Result<()> {
        check_unannotated_constructor_method_references(
            r#"
                local M = {}

                function M.open(path)
                    return setmetatable({ path = path }, { __index = M })
                end

                function M:cl<??>ose()
                end

                return M
            "#,
            7,
        )
    }

    #[gtest]
    fn test_method_references_through_nullable_receiver() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        ws.def_file("b.lua", "local h = io.open('z')\nh:close()\n");
        let (content, position) = check!(ProviderVirtualWorkspace::handle_file_content(
            r#"
                local f = assert(io.open('x'))
                f:cl<??>ose()
                local g = io.open('y')
                g:close()
            "#,
        ));
        let file_id = ws.def(&content);

        let result = references(&ws.analysis, file_id, position, false)
            .ok_or("failed to get references")
            .or_fail()?;

        let mut uses: Vec<(String, u32)> = result
            .iter()
            .filter(|l| !l.uri.as_str().ends_with("io.lua"))
            .map(|l| {
                let file = l.uri.as_str().rsplit('/').next().unwrap_or_default();
                (file.to_string(), l.range.start.line)
            })
            .collect();
        uses.sort();
        verify_eq!(
            uses,
            vec![
                ("b.lua".to_string(), 1),
                ("virtual_0.lua".to_string(), 2),
                ("virtual_0.lua".to_string(), 4),
            ]
        )
    }

    #[gtest]
    fn test_member_references_alias_cycle_does_not_stack_overflow() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();

        let (main_content, position) = check!(ProviderVirtualWorkspace::handle_file_content(
            r#"
                local t = {}
                t.m<??> = function() end
                local x = t.m
                t.m = x
            "#,
        ));
        let file_id = ws.def(&main_content);

        let result = references(&ws.analysis, file_id, position, true)
            .ok_or("failed to get references")
            .or_fail()?;

        let lines: HashSet<u32> = result.iter().map(|l| l.range.start.line).collect();
        assert!(lines.contains(&2));
        assert!(lines.contains(&3));
        assert!(lines.contains(&4));
        Ok(())
    }

    #[gtest]
    fn test_references_of_member_of_undefined_global_table() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
            r#"
                local a = vim.fn.std<??>path("data")
                local b = other.fn.stdpath("data")
                local c = vim.api.stdpath("data")
                local d = vim.fn.stdpath("cache")
            "#,
            vec![(
                "b.lua",
                r#"
                    vim.fn.stdpath("config")
                    local obj = make()
                    obj:stdpath()
                "#,
            )],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 1,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 4,
                },
                VirtualLocation {
                    file: "b.lua".to_string(),
                    line: 1,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_references_of_declared_member_ignore_same_name_on_undeclared_receiver() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
            r#"
                local M = {}
                function M.fi<??>nd() end
                M.find()
                other.find()
                vim.fn.find()
            "#,
            vec![],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 3,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_references_of_member_of_unresolved_local() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_references(
            r#"
                local h = dofile("harness.lua")
                h.che<??>ck(1)
                h:check(2)
                local g = dofile("harness.lua")
                g.check(3)
            "#,
            vec![(
                "b.lua",
                r#"
                    local h = dofile("harness.lua")
                    h.check(4)
                "#,
            )],
            vec![
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 2,
                },
                VirtualLocation {
                    file: "virtual_0.lua".to_string(),
                    line: 3,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_references_of_a_stdlib_function_leave_out_its_stub() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        let (content, position) = check!(ProviderVirtualWorkspace::handle_file_content(
            r#"
                local a = string.for<??>mat("%d", 1)
                local b = string.format("%d", 2)
            "#,
        ));
        let file_id = ws.def(&content);

        let result = references(&ws.analysis, file_id, position, true)
            .ok_or("failed to get references")
            .or_fail()?;

        let uris = result
            .iter()
            .map(|location| location.uri.as_str().to_string())
            .collect::<Vec<_>>();
        verify_that!(
            uris.iter().filter(|uri| uri.contains("virtual_")).count(),
            eq(2)
        )?;
        verify_that!(
            uris.iter().any(|uri| uri.contains("/resources/std/")),
            eq(false)
        )?;
        Ok(())
    }

    #[gtest]
    fn test_references_of_an_assigned_class_field_include_the_field_and_its_uses() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let (content, position) = check!(ProviderVirtualWorkspace::handle_file_content(
            r#"
                ---@class State
                ---@field version? integer

                ---@param s State
                local function clear(s)
                    s.vers<??>ion = nil
                end

                ---@param t State
                local function show(t)
                    print(t.version)
                end
            "#,
        ));
        let file_id = ws.def(&content);

        let result = references(&ws.analysis, file_id, position, true)
            .ok_or("failed to get references")
            .or_fail()?;

        let mut lines = result
            .iter()
            .map(|location| location.range.start.line)
            .collect::<Vec<_>>();
        lines.sort();
        verify_eq!(lines, vec![2, 6, 11])
    }

    #[gtest]
    fn test_references_of_a_require_field_held_in_a_table_constructor() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "util.lua",
            "local M = {}\nfunction M.create() end\nreturn M\n",
        );
        let (content, position) = check!(ProviderVirtualWorkspace::handle_file_content(
            r#"
                local lsp = { util = require 'util' }
                lsp.util.crea<??>te()
                lsp.util.create()
            "#,
        ));
        let file_id = ws.def(&content);

        let result = references(&ws.analysis, file_id, position, true)
            .ok_or("failed to get references")
            .or_fail()?;

        let mut lines = result
            .iter()
            .map(|location| location.range.start.line)
            .collect::<Vec<_>>();
        lines.sort();
        lines.dedup();
        verify_eq!(lines, vec![1, 2, 3])
    }
}
