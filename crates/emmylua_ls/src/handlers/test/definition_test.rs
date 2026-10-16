#[cfg(test)]
mod tests {
    use crate::handlers::test_lib::{ProviderVirtualWorkspace, VirtualLocation, check};
    use emmylua_code_analysis::{DocSyntax, Emmyrc};
    use googletest::prelude::*;

    type Expected = VirtualLocation;

    #[gtest]
    fn test_vararg_definition_uses_the_nearest_function() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local function outer(...)
                    local function inner(...)
                        return .<??>..
                    end
                    return inner(...)
                end
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 2
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_global_function_declared_in_several_files() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("a.lua", "function shared() end");
        ws.def_file("b.lua", "function shared() end");
        check!(ws.check_definition(
            r#"
                local function run()
                    sha<??>red()
                end
            "#,
            vec![
                Expected {
                    file: "b.lua".to_string(),
                    line: 0
                },
                Expected {
                    file: "a.lua".to_string(),
                    line: 0
                }
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_field_on_inline_require() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "mod_a.lua",
            r#"
                local M = {}
                function M.translate(s) return s end
                return M
            "#,
        );
        check!(ws.check_definition(
            r#"
                local tr = require("mod_a").trans<??>late
            "#,
            vec![Expected {
                file: "mod_a.lua".to_string(),
                line: 2
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_field_through_local_alias_of_global_table() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "lib.lua",
            r#"
                vim = {}
                vim.api = {}
                function vim.api.get_lines() end
            "#,
        );
        check!(ws.check_definition(
            r#"
                local api = vim.api
                api.get_<??>lines()
            "#,
            vec![Expected {
                file: "lib.lua".to_string(),
                line: 3
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_inside_array_element_closure() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local steps = {
                    function()
                        local x = 1
                        return <??>x + 1
                    end,
                }
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 3
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_stdlib_function_through_local_alias_of_global() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_definition(
            r#"
                local string = string
                local s = string.for<??>mat("%d", 1)
            "#,
            vec![Expected {
                file: "string.lua".to_string(),
                line: 118,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_field_through_local_alias_of_workspace_global() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "lib.lua",
            r#"
                helpers = {}
                function helpers.trim() end
            "#,
        );
        check!(ws.check_definition(
            r#"
                local h = helpers
                h.tr<??>im()
            "#,
            vec![Expected {
                file: "lib.lua".to_string(),
                line: 2
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_method_on_string_local() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_definition(
            r#"
                local s = "abc"
                local r = s:su<??>b(1, 2)
            "#,
            vec![Expected {
                file: "string.lua".to_string(),
                line: 317
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_field_on_call_returning_require() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "mod_a.lua",
            r#"
                local M = {}
                function M.translate(s) return s end
                return M
            "#,
        );
        check!(ws.check_definition(
            r#"
                local function lazy() return require("mod_a") end
                lazy().trans<??>late("x")
            "#,
            vec![Expected {
                file: "mod_a.lua".to_string(),
                line: 2
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_method_on_call_result() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                ---@class Reader
                local Reader = {}
                function Reader:close() end

                ---@return Reader
                local function open() return Reader end

                open():cl<??>ose()
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 3
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_param_lists_its_reassignments() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local function f(bufnr)
                    bufnr = tostring(bufnr)
                    return buf<??>nr
                end
            "#,
            vec![
                Expected {
                    file: "".to_string(),
                    line: 1,
                },
                Expected {
                    file: "".to_string(),
                    line: 2,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_local_lists_assignments_in_nested_closures() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local res
                run(function() res = 1 end)
                run(function() res = 2 end)
                print(re<??>s)
            "#,
            vec![
                Expected {
                    file: "".to_string(),
                    line: 1,
                },
                Expected {
                    file: "".to_string(),
                    line: 2,
                },
                Expected {
                    file: "".to_string(),
                    line: 3,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_local_alias_of_a_function_lists_origin_and_alias() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local M = {}
                function M.target() end
                local alias = M.target
                ali<??>as()
            "#,
            vec![
                Expected {
                    file: "".to_string(),
                    line: 2,
                },
                Expected {
                    file: "".to_string(),
                    line: 3,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_local_alias_of_a_stdlib_function() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_definition(
            r#"
                local format = string.format
                local s = for<??>mat("%d", 1)
            "#,
            vec![
                Expected {
                    file: "string.lua".to_string(),
                    line: 118,
                },
                Expected {
                    file: "".to_string(),
                    line: 1,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_local_alias_of_a_required_stdlib_function() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_definition(
            r#"
                local format = require("string").format
                local s = for<??>mat("%d", 1)
            "#,
            vec![
                Expected {
                    file: "string.lua".to_string(),
                    line: 118,
                },
                Expected {
                    file: "".to_string(),
                    line: 1,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_unreassigned_local_stays_single() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local value = 1
                print(val<??>ue)
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 1,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_field_lists_every_assignment() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local fs = {}
                fs.clone = 1
                fs.clone = 2
                print(fs.clo<??>ne)
            "#,
            vec![
                Expected {
                    file: "".to_string(),
                    line: 2,
                },
                Expected {
                    file: "".to_string(),
                    line: 3,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_field_skips_assignments_through_other_variables() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local t = {}
                local a = t
                local b = t
                a.x = 1
                b.x = 2
                print(a.<??>x)
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 4,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_self_method_on_untyped_param_table() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local N = {}
                function N.install(Reader)
                    function Reader:note_free(a) end
                    function Reader:scan() self:note_<??>free(1) end
                end
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 3,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_basic_definition() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                ---@generic T
                ---@param name `T`
                ---@return T
                local function new(name)
                    return name
                end

                ---@class Ability

                local a = new("<??>Ability")
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 8
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_table_field_definition_1() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                ---@class T
                ---@field func fun(self:string)

                ---@type T
                local t = {
                    f<??>unc = function(self)
                    end
                }
            "#,
            vec![
                Expected {
                    file: "".to_string(),
                    line: 2
                },
                Expected {
                    file: "".to_string(),
                    line: 6
                },
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_table_field_definition_2() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                ---@class T
                ---@field func fun(self: T) 注释注释

                ---@type T
                local t = {
                    func = function(self)
                    end,
                    a = 1,
                }

                t:func<??>()
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 2
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_goto_field() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local t = {}
                function t:test(a)
                    self.abc = a
                end

                print(t.abc<??>)
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 3
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_goto_label_definition() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                while true do
                    goto co<??>nt
                    ::cont::
                end
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 3
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_goto_overload() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "test.lua",
            r#"
                ---@class Goto1
                ---@class Goto2
                ---@class Goto3

                ---@class T
                ---@field func fun(a:Goto1) # 1
                ---@field func fun(a:Goto2) # 2
                ---@field func fun(a:Goto3) # 3
                local T = {}

                function T:func(a)
                end
            "#,
        );

        check!(ws.check_definition(
            r#"
                ---@type Goto2
                local Goto2

                ---@type T
                local t
                t.fu<??>nc(Goto2)
             "#,
            vec![
                Expected {
                    file: "test.lua".to_string(),
                    line: 6,
                },
                Expected {
                    file: "test.lua".to_string(),
                    line: 7,
                },
            ]
        ));

        check!(ws.check_definition(
            r#"
                ---@type T
                local t
                t.fu<??>nc()
             "#,
            vec![
                Expected {
                    file: "test.lua".to_string(),
                    line: 6,
                },
                Expected {
                    file: "test.lua".to_string(),
                    line: 7,
                },
                Expected {
                    file: "test.lua".to_string(),
                    line: 8,
                },
                Expected {
                    file: "test.lua".to_string(),
                    line: 11,
                },
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_goto_return_field() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "test.lua",
            r#"
                local function test()

                end

                return {
                    test = test,
                }
            "#,
        );
        check!(ws.check_definition(
            r#"
                local t = require("test")
                local test = t.test
                te<??>st()
            "#,
            vec![
                VirtualLocation {
                    file: "test.lua".to_string(),
                    line: 1
                },
                VirtualLocation {
                    file: "".to_string(),
                    line: 2
                }
            ]
        ));

        Ok(())
    }

    #[gtest]
    fn test_goto_return_field_2() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "test.lua",
            r#"
                ---@class Export
                local export = {}
                ---@generic T
                ---@param name `T`|T
                ---@param tbl? table
                ---@return T
                local function new(name, tbl)
                end

                export.new = new
                return export
            "#,
        );
        check!(ws.check_definition(
            r#"
                local new = require("test").new
                new<??>("A")
            "#,
            vec![
                Expected {
                    file: "test.lua".to_string(),
                    line: 7
                },
                Expected {
                    file: "".to_string(),
                    line: 1
                }
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_goto_require_return_table_fallback() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "reported-tasks.lua",
            r#"
            ---@class TestModule
            local TestModule = {}
            function TestModule:getA()
            end
            return {
                TestModule = TestModule,
            }
        "#,
        );

        check!(ws.check_definition(
            r#"
                local reportedTasks = require("reported-tasks")
                reported<??>Tasks.TestModule:getA()
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 1,
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_goto_generic_type() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "1.lua",
            r#"
                ---@generic T
                ---@param name `T`|T
                ---@return T
                function new(name)
                end
            "#,
        );
        ws.def_file(
            "2.lua",
            r#"
                ---@namespace AAA
                ---@class BBB<T>
            "#,
        );
        check!(ws.check_definition(
            r#"
                new("AAA.BBB<??>")
            "#,
            vec![Expected {
                file: "2.lua".to_string(),
                line: 2
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_goto_export_function() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "a.lua",
            r#"
                local function create()
                end

                return create
            "#,
        );
        check!(ws.check_definition(
            r#"
                local create = require('a')
                create<??>()
            "#,
            vec![
                Expected {
                    file: "a.lua".to_string(),
                    line: 1
                },
                Expected {
                    file: "".to_string(),
                    line: 1
                }
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_goto_export_function_2() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "a.lua",
            r#"
                local function testA()
                end

                local function create()
                end

                return create
            "#,
        );
        ws.def_file(
            "b.lua",
            r#"
                local Rxlua = {}
                local create = require('a')

                Rxlua.create = create
                return Rxlua
            "#,
        );
        check!(ws.check_definition(
            r#"
                local create = require('b').create
                create<??>()
            "#,
            vec![
                Expected {
                    file: "a.lua".to_string(),
                    line: 4
                },
                Expected {
                    file: "".to_string(),
                    line: 1
                }
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_doc_resolve() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();

        let mut emmyrc = Emmyrc::default();
        emmyrc.doc.syntax = DocSyntax::Myst;
        ws.analysis.update_config(emmyrc.into());

        ws.def_file(
            "a.lua",
            r#"
                --- @class X
                --- @field a string

                --- @class ns.Y
                --- @field b string
            "#,
        );

        check!(ws.check_definition(
            r#"
                --- {lua:obj}`X<??>`
            "#,
            vec![Expected {
                file: "a.lua".to_string(),
                line: 1
            }]
        ));

        check!(ws.check_definition(
            r#"
                --- {lua:obj}`X<??>.a`
            "#,
            vec![Expected {
                file: "a.lua".to_string(),
                line: 1
            }]
        ));

        check!(ws.check_definition(
            r#"
                --- {lua:obj}`X.a<??>`
            "#,
            vec![Expected {
                file: "a.lua".to_string(),
                line: 2
            }]
        ));

        check!(ws.check_definition(
            r#"
                --- @using ns

                --- {lua:obj}`X<??>`
            "#,
            vec![Expected {
                file: "a.lua".to_string(),
                line: 1
            }]
        ));

        check!(ws.check_definition(
            r#"
                --- @using ns

                --- {lua:obj}`Y<??>`
            "#,
            vec![Expected {
                file: "a.lua".to_string(),
                line: 4
            }]
        ));

        check!(ws.check_definition(
            r#"
                --- @using ns

                --- {lua:obj}`ns.Y<??>`
            "#,
            vec![Expected {
                file: "a.lua".to_string(),
                line: 4
            }]
        ));

        check!(ws.check_definition(
            r#"
                --- {lua:obj}`c<??>`
                --- @class Z
                --- @field c string
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 3
            }]
        ));

        Ok(())
    }

    #[gtest]
    fn test_goto_variable_param() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "a.lua",
            r#"
                ---@class Observable<T>

                ---test
                local function zipLatest(...)
                end
                return zipLatest
            "#,
        );
        ws.def_file(
            "b.lua",
            r#"
            local export = {}
            local zipLatest = require('a')
            export.zipLatest = zipLatest
            return export
            "#,
        );
        check!(ws.check_definition(
            r#"
                local zipLatest = require('b').zipLatest
                zipLatest<??>()
            "#,
            vec![
                Expected {
                    file: "a.lua".to_string(),
                    line: 4,
                },
                Expected {
                    file: "".to_string(),
                    line: 1,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_goto_see() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let mut emmyrc = Emmyrc::default();
        emmyrc.doc.syntax = DocSyntax::Myst;
        ws.analysis.update_config(emmyrc.into());

        ws.def_file(
            "a.lua",
            r#"
                ---@class Meep
            "#,
        );

        check!(ws.check_definition(
            r#"
                --- @see Mee<??>p
            "#,
            vec![Expected {
                file: "a.lua".to_string(),
                line: 1,
            }],
        ));

        check!(ws.check_definition(
            r#"
                --- @class Foo
                --- @field bar int
                local Foo = {}

                --- @see b<??>ar
                Foo.xxx = 0
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 2,
            }],
        ));

        Ok(())
    }

    #[gtest]
    fn test_accessors() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
            ---@class A
            ---@[field_accessor]
            ---@field age number
            local A

            ---@private
            function A:getAge()
            end

            ---@private
            function A:setAge(value)
            end
            "#,
        );

        check!(ws.check_definition(
            r#"
                ---@type A
                local obj
                obj.age<??> = 1
            "#,
            vec![
                Expected {
                    file: "".to_string(),
                    line: 3,
                },
                Expected {
                    file: "".to_string(),
                    line: 3,
                },
                Expected {
                    file: "".to_string(),
                    line: 7,
                },
                Expected {
                    file: "".to_string(),
                    line: 11,
                }
            ],
        ));

        Ok(())
    }

    #[gtest]
    fn test_intersection() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
        ---@class Matchers
        ---@field toBe fun(expected: any)

        ---@class Inverse
        ---@field negate number
        "#,
        );
        check!(ws.check_definition(
            r#"
            ---@type Matchers & Inverse
            local a
            a.ne<??>gate = 0
            "#,
            vec![
                Expected {
                    file: "".to_string(),
                    line: 3,
                },
                Expected {
                    file: "".to_string(),
                    line: 5,
                },
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_string_key_in_table_constructor() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local t = {
                    ["unt<??>il"] = 1,
                }
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 2,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_string_key_use_finds_constructor_field() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local t = {
                    ["until"] = 1,
                    name = 2,
                }
                print(t["na<??>me"])
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 3,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_dotted_use_finds_string_key_field() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local t = {
                    ["name"] = 1,
                }
                print(t.na<??>me)
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 2,
            }],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_member_of_a_table_returned_by_an_iife() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local M = {}
                M.path = (function()
                    local function iterate_parents(p) return p end
                    return { iterate_parents = iterate_parents }
                end)()
                print(M.path.iterate_<??>parents(1))
            "#,
            vec![
                Expected {
                    file: "".to_string(),
                    line: 3,
                },
                Expected {
                    file: "".to_string(),
                    line: 4,
                }
            ],
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_require_field_held_in_a_table_constructor() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "util.lua",
            r#"
                local M = {}
                function M.create() end
                return M
            "#,
        );
        check!(ws.check_definition(
            r#"
                local lsp = { util = require 'util' }
                lsp.util.crea<??>te()
            "#,
            vec![Expected {
                file: "util.lua".to_string(),
                line: 2
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_through_global_field_assigned_a_required_module() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "lsp_mod.lua",
            r#"
                local lsp = {}
                function lsp.get_client() end
                return lsp
            "#,
        );
        ws.def_file(
            "glob.lua",
            r#"
                vim = {}
                vim.lsp = require("lsp_mod")
            "#,
        );
        check!(ws.check_definition(
            r#"
                vim.lsp.get_<??>client()
            "#,
            vec![Expected {
                file: "lsp_mod.lua".to_string(),
                line: 2
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_through_global_field_assigned_a_deferred_module() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "defer.lua",
            r#"
                ---@generic T
                ---@param root string
                ---@param mod T
                ---@return T
                function defer(root, mod)
                    return setmetatable({ _sub = mod }, { __index = function() end })
                end
            "#,
        );
        ws.def_file(
            "lsp_mod.lua",
            r#"
                local lsp = defer("x", { a = 1 })
                function lsp.get_client() end
                return lsp
            "#,
        );
        ws.def_file(
            "glob.lua",
            r#"
                vim = {}
                vim.lsp = require("lsp_mod")
            "#,
        );
        check!(ws.check_definition(
            r#"
                vim.lsp.get_<??>client()
            "#,
            vec![Expected {
                file: "lsp_mod.lua".to_string(),
                line: 2
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_a_required_field_assigned_inside_a_callback() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "flag_mod.lua",
            r#"
                local M = {
                    flag = false,
                }
                return M
            "#,
        );
        check!(ws.check_definition(
            r#"
                local m = require("flag_mod")
                describe("x", function()
                    before_each(function()
                        m.flag = true
                    end)
                    after_each(function()
                        m.fl<??>ag = false
                    end)
                end)
            "#,
            vec![
                Expected {
                    file: "flag_mod.lua".to_string(),
                    line: 2
                },
                Expected {
                    file: "".to_string(),
                    line: 4
                },
                Expected {
                    file: "".to_string(),
                    line: 7
                },
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_a_key_of_a_call_argument_table_lists_the_class_field() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                ---@class SetOpts
                ---@field hl_group? string

                ---@param opts SetOpts
                local function set(buf, opts) end

                set(1, { hl_g<??>roup = "A" })
            "#,
            vec![
                Expected {
                    file: "".to_string(),
                    line: 2
                },
                Expected {
                    file: "".to_string(),
                    line: 7
                },
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_an_aliased_function_lists_its_overload_lines() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "ov_mod.lua",
            r#"
                local M = {}

                ---@param a integer
                ---@overload fun(a: string): string
                function M.f(a, b) end

                return M
            "#,
        );
        check!(ws.check_definition(
            r#"
                local f = require("ov_mod").<??>f
            "#,
            vec![
                Expected {
                    file: "ov_mod.lua".to_string(),
                    line: 4
                },
                Expected {
                    file: "ov_mod.lua".to_string(),
                    line: 5
                },
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_a_called_function_omits_its_overload_lines() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "ov_mod2.lua",
            r#"
                local M = {}

                ---@param a integer
                ---@overload fun(a: string): string
                function M.f(a, b) end

                return M
            "#,
        );
        check!(ws.check_definition(
            r#"
                require("ov_mod2").<??>f(1)
            "#,
            vec![Expected {
                file: "ov_mod2.lua".to_string(),
                line: 5
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_a_global_function_defined_in_several_files() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("shout_a.lua", "function shout(x) end\nshout(1)\n");
        ws.def_file("shout_b.lua", "\n\nfunction shout(x) end\n");
        check!(ws.check_definition(
            r#"
                -- a standalone script with its own copy
                -- of the helper
                function shout(x) end
                sho<??>ut(2)
            "#,
            vec![
                Expected {
                    file: "shout_a.lua".to_string(),
                    line: 0
                },
                Expected {
                    file: "shout_b.lua".to_string(),
                    line: 2
                },
                Expected {
                    file: "".to_string(),
                    line: 3
                },
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_a_field_assigned_through_self_in_a_method() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            r#"
                local Context = {
                    last_request_time = nil,
                }

                function Context:reset()
                    self.last_request_time = nil
                end

                local function update(now)
                    Context.last_<??>request_time = now
                end
            "#,
            vec![
                Expected {
                    file: "".to_string(),
                    line: 2
                },
                Expected {
                    file: "".to_string(),
                    line: 6
                },
                Expected {
                    file: "".to_string(),
                    line: 10
                },
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_class_name_keeps_blocks_of_other_files_after_one_is_updated() -> Result<()>
    {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("a.lua", "---@class view.Shared\nlocal A = {}\n");
        ws.def_file("b.lua", "---@class view.Shared\nlocal B = {}\n");
        // An open or edited file is re-indexed; the other files' blocks of the class stay.
        ws.def_file("b.lua", "---@class view.Shared\nlocal B = {}\n");
        check!(ws.check_definition(
            r#"
                ---@class vi<??>ew.Shared
                local C = {}
            "#,
            vec![
                Expected {
                    file: "a.lua".to_string(),
                    line: 0
                },
                Expected {
                    file: "b.lua".to_string(),
                    line: 0
                },
                Expected {
                    file: "".to_string(),
                    line: 1
                },
            ]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_method_defined_in_another_block_of_the_same_class() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "one.lua",
            "---@class view.Shared\nlocal One = {}\n\nfunction One:first() end\n",
        );
        ws.def_file(
            "two.lua",
            "---@class view.Shared\nlocal Two = {}\n\nfunction Two:second() end\n",
        );
        check!(ws.check_definition(
            r#"
                ---@type view.Shared
                local shared

                shared:sec<??>ond()
            "#,
            vec![Expected {
                file: "two.lua".to_string(),
                line: 3
            }]
        ));
        Ok(())
    }

    #[gtest]
    fn test_definition_of_self_field_when_another_block_of_the_class_defines_the_same_method()
    -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "narrow.lua",
            "---@class view.Menu\nlocal narrow = {}\n\nnarrow.open = function(self, offset)\n    return offset\nend\n",
        );
        check!(ws.check_definition(
            r#"
                ---@class view.Menu
                ---@field width integer
                local wide = {}

                wide.open = function(self, offset)
                    return self.wi<??>dth + offset
                end
            "#,
            vec![Expected {
                file: "".to_string(),
                line: 2
            }]
        ));
        Ok(())
    }
}
