#[cfg(test)]
mod tests {
    use crate::handlers::test_lib::{ProviderVirtualWorkspace, VirtualHoverResult, check};
    use googletest::prelude::*;

    fn dedent(input: &str) -> String {
        let lines: Vec<&str> = input.lines().collect();
        let mut min_indent = usize::MAX;
        for line in &lines {
            if line.trim().is_empty() {
                continue;
            }
            let indent = line.chars().take_while(|c| *c == ' ').count();
            min_indent = min_indent.min(indent);
        }
        if min_indent == usize::MAX {
            return String::new();
        }
        let mut out = String::new();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = if line.len() >= min_indent {
                &line[min_indent..]
            } else {
                line
            };
            out.push_str(trimmed);
            if i + 1 < lines.len() {
                out.push('\n');
            }
        }
        out.trim_start_matches('\n').trim_end().to_string()
    }

    #[gtest]
    fn test_1() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class <??>A
                ---@field a number
                ---@field b string
                ---@field c boolean
            "#,
            VirtualHoverResult {
                value:
                    "```lua\n(class) A {\n    a: number,\n    b: string,\n    c: boolean,\n}\n```"
                        .to_string(),
            },
        ));
        Ok(())
    }

    /// `M.__index = M` makes the table contain itself; the field renders it once, not nested.
    #[gtest]
    fn test_hover_self_referencing_table_field_is_not_nested() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local M = {}
                M.__index = M

                function M.new() end

                local x = M.__<??>index
            "#,
            VirtualHoverResult {
                value:
                    "```lua\n(field) M.__index: { __index: table, new: function }\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_class_index_signature() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Foo
                ---@field [integer] string

                ---@type Foo
                local <??>foo
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal foo: Foo {\n    [integer]: string,\n}\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_class_nil_type_key_hidden() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class <??>AAA
                ---@field [true] 1
                ---@field [nil] 2
            "#,
            VirtualHoverResult {
                value: "```lua\n(class) AAA {\n    [true]: integer = 1,\n}\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_right_to_left() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        // check!(ws.check_hover(
        //     r#"
        //         ---@class H4
        //         local m = {
        //             x = 1
        //         }

        //         ---@type H4
        //         local m1

        //         m1.x = {}
        //         m1.<??>x = {}
        //     "#,
        //     VirtualHoverResult {
        //         value: "```lua\n(field) x: integer = 1\n```".to_string(),
        //     },
        // ));

        check!(ws.check_hover(
            r#"
                ---@class Node
                ---@field x number
                ---@field right Node?

                ---@return Node
                local function createRBNode()
                end

                ---@type Node
                local node

                if node.right then
                else
                    node.<??>right = createRBNode()
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) Node.right: Node {\n    x: number,\n    right: Node?,\n}\n```"
                    .to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                 ---@class Node1
                ---@field x number

                ---@return Node1
                local function createRBNode()
                end

                ---@type Node1?
                local node

                if node then
                else
                    <??>node = createRBNode()
                end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal node: Node1 {\n    x: number,\n}\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_nil() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class A
                ---@field a? number

                ---@type A
                local a

                local d = a.<??>a
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) A.a: number?\n```".to_string(),
            },
        ));
        Ok(())
    }

    #[gtest]
    fn test_function_infer_return_val() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local function <??>f(a, b)
                    a = 1
                end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function f(a, b)\n```".to_string(),
            },
        ));
        Ok(())
    }

    #[gtest]
    fn test_hover_param_string() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@param n string doc
                function foo(<??>n)
                end
            "#,
            VirtualHoverResult {
                value: dedent(
                    r#"
                    ```lua
                    (parameter) n: string
                    ```

                    ---

                    doc
                    "#
                )
            },
        ));
        Ok(())
    }

    #[gtest]
    fn test_hover_param_func() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@param n fun():boolean doc
                function foo(<??>n)
                end
            "#,
            VirtualHoverResult {
                value: dedent(
                    r#"
                    ```lua
                    (parameter) n: fun() -> boolean
                    ```

                    ---

                    doc
                    "#
                )
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_generic_param_constraint_and_field_description() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Animal
                ---@field name string 名字
                ---@field age integer 年龄

                ---@generic T: Animal
                ---@param animal T
                function checkAnimal(<??>animal)
                    print(animal.age)
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) animal: T extends Animal\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@class Animal
                ---@field name string 名字
                ---@field age integer 年龄

                ---@generic T: Animal
                ---@param animal T
                function checkAnimal(animal)
                    print(animal.<??>age)
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) animal.age: integer\n```\n\n---\n\n年龄".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@class Animal
                ---@field name string 名字
                ---@field age integer 年龄

                ---@generic T: Animal
                ---@param animal T?
                function checkAnimal(<??>animal)
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) animal: T?\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_special_alias_call_type_syntax() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class KeyofHoverShape
                ---@field name string

                ---@type keyof KeyofHoverShape
                local <??>key
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal key: \"name\"\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@class IndexHoverShape
                ---@field name string

                ---@type IndexHoverShape["name"]
                local <??>value
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal value: string\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@class GenericIndexHoverShape
                ---@field name string

                ---@alias GenericIndexHoverPick<K extends keyof T, T> T[K]

                ---@type GenericIndexHoverPick<"name", GenericIndexHoverShape>
                local <??>value
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal value: string\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@class ExtendsHoverShape
                ---@field name string

                ---@type ExtendsHoverShape extends table and number or string
                local <??>is_table
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal is_table: number\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@alias AB<??>C<K extends keyof T, T> T[K]
            "#,
            VirtualHoverResult {
                value: "```lua\n(alias) ABC<K extends keyof T, T> = T[K]\n```".to_string(),
            },
        ));

        check!(
            ws.check_hover(
                r#"
                ---@alias Co<??>py<T> { readonly [K in keyof T]?: T[K] }
            "#,
                VirtualHoverResult {
                    value: "```lua\n(alias) Copy<T> = { readonly [K in keyof T]?: T[K]; }\n```"
                        .to_string(),
                },
            )
        );

        check!(
            ws.check_hover(
                r#"
                ---@class BoxHoverShape
                ---@field name string

                ---@class BoxHoverShape<??>Box<K extends keyof BoxHoverShape, T>
                ---@field value T
            "#,
                VirtualHoverResult {
                    value:
                        "```lua\n(class) BoxHoverShapeBox<K extends keyof BoxHoverShape, T>\n```"
                            .to_string(),
                },
            )
        );
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_narrowed_function_type() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@param n integer|fun():boolean
                function _G.foo(n)
                    local f = n
                    if type(f) ~= 'function' then
                        f = function()
                            return true
                        end
                    end
                    local _ = <??>f
                end
            "#,
            VirtualHoverResult {
                value: dedent(
                    r#"
                    ```lua
                    (parameter) n: fun() -> boolean
                    ```
                    "#
                ),
            },
        ));
        Ok(())
    }

    /// An explicit `self` first parameter of `function Class.method(self)` is the owner.
    #[gtest]
    fn test_hover_explicit_self_parameter_is_owner() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Counter
                ---@field count integer
                local Counter = {}

                function Counter.bump(<??>self)
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) self: Counter {\n    count: integer,\n    bump: function,\n}\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// An annotated `self` parameter keeps its annotation.
    #[gtest]
    fn test_hover_annotated_explicit_self_parameter_wins() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Counter
                local Counter = {}

                ---@param self string
                function Counter.bump(<??>self)
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) self: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_decl_desc() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Buff.AddData
                ---@field pulse? number 心跳周期

                ---@type Buff.AddData
                local data

                data.pu<??>lse
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) Buff.AddData.pulse: number?\n```\n\n&nbsp;&nbsp;in class `Buff.AddData`\n\n---\n\n心跳周期".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_issue_535() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@type table<string, number>
                local t

                ---@class T1
                local a

                function a:init(p)
                    self._c<??>fg = t[p]
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) T1._cfg: number\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@type table<string, number>
                local t = {}
                ---@class T2
                local a = {}

                function a:init(p)
                    self._cfg = t[p]
                end

                ---@param p T2
                function fun(p)
                    local x = p._c<??>fg
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) T2._cfg: number\n```".to_string(),
            },
        ));
    Ok(())
    }

    #[gtest]
    fn test_signature_desc() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                -- # A
                local function a<??>bc()
                end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function abc()\n```\n\n---\n\n# A".to_string(),
            },
        ));
        Ok(())
    }

    #[gtest]
    fn test_class_desc() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---A1
                ---@class AB<??>C
                ---A2
            "#,
            VirtualHoverResult {
                value: "```lua\n(class) ABC\n```\n\n---\n\nA1".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_class_bound_local_decl_description() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                --- This is the MyModule documentation.
                --- It should appear when hovering over MyModule.
                --- @class MyModule
                local My<??>Module
            "#,
            VirtualHoverResult {
                value: dedent(
                    r#"
                    ```lua
                    local MyModule: MyModule
                    ```

                    ---

                    This is the MyModule documentation.
                    It should appear when hovering over MyModule.
                    "#,
                ),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_class_bound_member_description() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                --- @class MyModule
                local MyModule = {}

                --- This is the SubModule documentation.
                --- It should appear when hovering over SubModule.
                --- @class MyModule.SubModule
                MyModule.Sub<??>Module = {}
            "#,
            VirtualHoverResult {
                value: dedent(
                    r#"
                    ```lua
                    (field) MyModule.SubModule: MyModule.SubModule
                    ```

                    ---

                    This is the SubModule documentation.
                    It should appear when hovering over SubModule.
                    "#,
                ),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_attribute_hover_uses_arg_types() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class custom_attribute: Attribute
                ---@overload fun(value: string)
                ---@overload fun(value: integer)

                ---@[custom_at<??>tribute(1)]
                local a
            "#,
            VirtualHoverResult {
                value: "```lua\n(class) custom_attribute(value: integer)\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_alias_desc() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@alias Tes<??>Alias
                ---| 'A' # A1
                ---| 'B' # A2
            "#,
            VirtualHoverResult {
                value: "```lua\n(alias) TesAlias = (\"A\"|\"B\")\n    | \"A\" -- A1\n    | \"B\" -- A2\n\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_type_desc() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local export = {
                    ---@type number? activeSub
                    vvv = nil
                }

                export.v<??>vv
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) export.vvv: number?\n```\n\n---\n\nactiveSub".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_field_key() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
                ---@class ObserverParams
                ---@field next fun() # 测试

                ---@param params fun() | ObserverParams
                function test(params)
                end
            "#,
        );
        check!(ws.check_hover(
            r#"
                test({
                    <??>next = function()
                    end
                })
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) ObserverParams.next()\n```\n\n---\n\n测试".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_field_key_for_generic() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
                ---@class ObserverParams<T>
                ---@field next fun() # 测试

                ---@generic T
                ---@param params fun() | ObserverParams<T>
                function test(params)
                end
            "#,
        );
        check!(ws.check_hover(
            r#"
                test({
                    <??>next = function()
                    end
                })
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) ObserverParams.next()\n```\n\n---\n\n测试".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_before_dot_returns_object_info() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
                ---@class Node
                ---@field field number?
                ---@field method fun(self: Node)

                ---@type Node
                node = {}

                function node.method() end
            "#,
        );

        check!(ws.check_hover(
            r#"
                node<??>.field = nil
            "#,
            VirtualHoverResult {
                value: "```lua\n(global) node: Node {\n    field: number?,\n    method: function,\n}\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                node<??>:method()
            "#,
            VirtualHoverResult {
                value: "```lua\n(global) node: Node {\n    field: number?,\n    method: function,\n}\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                node<??>["key"] = "value"
            "#,
            VirtualHoverResult {
                value: "```lua\n(global) node: Node {\n    field: number?,\n    method: function,\n}\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                node["key"<??>] = "value"
            "#,
            VirtualHoverResult {
                value: "\"key\"".to_string(),
            },
        ));

        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_see_tag() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                --- Description
                ---
                --- @see a.b.c
                local function te<??>st() end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function test()\n```\n\n---\n\nDescription\n\n---\n\n@*see* a.b.c".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                --- Description
                ---
                --- @see a.b.c see description
                local function te<??>st() end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function test()\n```\n\n---\n\nDescription\n\n---\n\n@*see* a.b.c see description".to_string(),
            },
        ));

        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_other_tag() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                --- Description
                ---
                --- @xyz content
                local function te<??>st() end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function test()\n```\n\n---\n\nDescription\n\n---\n\n@*xyz* content".to_string(),
            },
        ));

        Ok(())
    }

    /// Indexing `table<K, V>` yields `V` for a matching key, however the key is written.
    #[gtest]
    fn test_hover_table_generic_index() -> Result<()> {
        let cases = [
            ("table<string, integer>", "t.foo"),
            ("table<string, integer>", "t[\"foo\"]"),
            ("table<integer, integer>", "t[1]"),
            ("table<string, integer>", "t[key]"),
        ];
        for (table_type, access) in cases {
            let mut ws = ProviderVirtualWorkspace::new();
            let code = format!(
                "---@type {table_type}\nlocal t = {{}}\n---@type any\nlocal key\nlocal <??>v = {access}\n"
            );
            check!(
                ws.check_hover(
                    &code,
                    VirtualHoverResult {
                        value: "```lua\nlocal v: integer\n```".to_string(),
                    },
                ),
                "{access} on {table_type}"
            );
        }
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_class_with_nil() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
            ---@class A
            ---@field aAnnotation? string a标签

            ---@class B
            ---@field bAnnotation? string b标签
            "#,
        );
        check!(ws.check_hover(
            r#"
            ---@type A|B|nil
            local defaultOpt = {
                aAnnota<??>tion = "a",
            }
            "#,
            VirtualHoverResult {
                value:
                    "```lua\n(field) aAnnotation: string = \"a\"\n```\n\n---\n\na标签".to_string(),
            },
        ));

        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_right_expr() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local max = 0
                local key
                if type(key) ~= "number" then
                    return false
                end
                m<??>ax = key
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal max: integer\n```".to_string(),
            },
        ));

        Ok(())
    }

    /// A field of a generic class must be projected through the prefix's type arguments at the use
    /// site: `@field aaa T` + `MyTable<int>` shows `integer`, not the declaration-site `T`.
    #[gtest]
    fn test_hover_generic_field_access_projected() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class MyTable<T>
                ---@field aaa T
                ---@field bbb T

                local c ---@type MyTable<int>

                print(c.<??>aaa)
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) MyTable.aaa: integer\n```".to_string(),
            },
        ));
        check!(ws.check_hover(
            r#"
                ---@class MyTable<T>
                ---@field aaa T
                ---@field bbb T

                local c ---@type MyTable<int>

                local x = c.<??>bbb
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) MyTable.bbb: integer\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// `#t` is an integer, not the type of `t`.
    #[gtest]
    fn test_hover_length_operator_is_integer() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@type string[]
                local items = {}

                local <??>count = #items
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal count: integer\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// Indexing an array with `#items` yields the element type.
    #[gtest]
    fn test_hover_array_indexed_by_length() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@type string[]
                local items = {}

                local <??>last = items[#items]
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal last: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// Generic arguments keep their parameter positions: `Pair<int, string>.second` is `string`.
    #[gtest]
    fn test_hover_generic_field_access_projected_multi_param() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Pair<T, U>
                ---@field first T
                ---@field second U

                local p ---@type Pair<int, string>

                print(p.<??>second)
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) Pair.second: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A method called on a nullable receiver (`file?`) resolves against the non-nil member.
    #[gtest]
    fn test_hover_method_on_nullable_receiver() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Box
                local Box = {}

                ---@return integer
                function Box:size() end

                ---@type Box?
                local b

                b:si<??>ze()
            "#,
            VirtualHoverResult {
                value: "```lua\n(method) Box:size() -> integer\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// `local lsp = vim.lsp` where `vim.lsp = require(...)` in another file: the alias reaches the module's members.
    #[gtest]
    fn test_hover_call_through_local_alias_of_global_field_holding_a_module() -> Result<()> {
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

                ---@return string
                function lsp.name() end
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
        check!(ws.check_hover(
            r#"
                local lsp = vim.lsp
                local n<??> = lsp.name()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal n: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A call through a global field that holds a required module keeps the declared return.
    #[gtest]
    fn test_hover_call_through_global_field_holding_a_deferred_module() -> Result<()> {
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

                ---@return string
                function lsp.name() end
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
        check!(ws.check_hover(
            r#"
                local n<??> = vim.lsp.name()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal n: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// `---@type uv` on `vim.uv = ...` in a meta file: the class's functions are reachable
    /// from another file.
    #[gtest]
    fn test_hover_call_through_global_field_with_declared_class() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "uv_meta.lua",
            r#"
                ---@meta
                ---@class uv
                local uv = {}

                ---@return string
                function uv.fs_name() end
            "#,
        );
        ws.def_file(
            "vim_meta.lua",
            r#"
                ---@meta
                vim = {}

                ---@type uv
                vim.uv = ...
            "#,
        );
        check!(ws.check_hover(
            r#"
                local n<??> = vim.uv.fs_name()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal n: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// `{}` assigned to a global declared `---@type K` stays a `K`: the class's runtime
    /// methods remain reachable.
    #[gtest]
    fn test_hover_method_of_global_declared_class_assigned_empty_table() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "k.lua",
            r#"
                ---@class K
                local K = {}

                ---@return string
                function K.name() end
            "#,
        );
        check!(ws.check_hover(
            r#"
                ---@type K
                shared = {}
                local n<??> = shared.name()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal n: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A module-level `local conn` assigned in another function is seen by later functions as
    /// the type of those assignments.
    #[gtest]
    fn test_hover_method_on_upvalue_assigned_in_another_function() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Conn
                local Conn = {}

                ---@return string
                function Conn:name() end

                ---@return Conn
                local function connect() end

                local conn
                local function setup()
                    conn = connect()
                end

                local function use()
                    assert(conn)
                    local n<??> = conn:name()
                end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal n: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A function literal called in place (`(function() ... end)()`) gives its return type.
    #[gtest]
    fn test_hover_member_of_immediately_invoked_function_result() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local M = {}
                M.path = (function()
                    return {
                        join = function()
                            return "x"
                        end,
                    }
                end)()
                local r<??> = M.path.join()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal r: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A guard that bails out with `return error(..)` does not make that call the module's export.
    #[gtest]
    fn test_hover_call_of_module_with_early_error_return() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "dns.lua",
            r#"
                local _M = {}

                local cache = nil
                do
                    if not cache then
                        return error("failed to create the cache")
                    end
                end

                ---@return string
                function _M.lookup(host) end

                return _M
            "#,
        );
        check!(ws.check_hover(
            r#"
                local r<??> = require("dns").lookup("x")
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal r: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A function literal in parentheses bound to a local is callable.
    #[gtest]
    fn test_hover_call_of_parenthesized_function_literal_local() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local make = (function()
                    return "s"
                end)
                local r<??> = make()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal r: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// `_M.count = count` (a local function) exported from a module keeps its signature when read through `require`.
    #[gtest]
    fn test_hover_required_member_assigned_a_local_function() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "util.lua",
            r#"
                local _M = {}

                ---@param t table
                ---@return integer
                local function count(t)
                    return 1
                end
                _M.count = count

                return _M
            "#,
        );
        check!(ws.check_hover(
            r#"
                local n = require("util").cou<??>nt
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function count(t: table) -> integer\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// `require("string")` yields the standard library table.
    #[gtest]
    fn test_hover_call_through_required_standard_library() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_hover(
            r#"
                local n<??> = require("string").upper("x")
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal n: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A string method called on `string?` or `string|number` still gives its result type.
    #[gtest]
    fn test_hover_string_method_on_nullable_and_union_receivers() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_hover(
            r#"
                ---@type string?
                local maybe
                local up<??> = maybe:upper()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal up: string\n```".to_string(),
            },
        ));
        check!(ws.check_hover(
            r#"
                ---@type string|number
                local either
                local up<??> = either:upper()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal up: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// No component of the receiver has the method: the result stays unknown rather than guessed.
    #[gtest]
    fn test_hover_method_missing_on_every_union_component_stays_unknown() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_hover(
            r#"
                ---@type number|boolean
                local neither
                local up<??> = neither:upper()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal up: unknown\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// `file:read()` returns `string|number|nil`, so string methods on its result resolve.
    #[gtest]
    fn test_hover_string_method_on_file_read_result() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_hover(
            r#"
                local h = io.popen("ls")
                local out = h:read("*a")
                local up<??> = out:upper()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal up: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A table literal inside a table literal is a plain `table`; only the outer level is expanded.
    #[gtest]
    fn test_hover_nested_table_literal_field_is_table() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local x<??> = { a = { b = 1 }, c = 2, d = { 1, 2 } }
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal x: { a: table, c: integer = 2, d: table }\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// Hover lists every field of a table literal instead of cutting the list short.
    #[gtest]
    fn test_hover_table_literal_lists_every_field() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local x<??> = { a = 1, b = 2, c = 3, d = 4, e = 5, f = 6, g = "s", h = true }
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal x: { a: integer = 1, b: integer = 2, c: integer = 3, d: integer = 4, e: integer = 5, f: integer = 6, g: \"s\", h: true }\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A function returning a table literal shows `table`, not the literal's members.
    #[gtest]
    fn test_hover_function_returning_table_literal() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local function m<??>k()
                    return { a = 1, b = "x" }
                end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function mk() -> table\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// Hovering a call through a nested global table keeps the function's return type.
    #[gtest]
    fn test_hover_call_through_nested_global_keeps_return() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                gv = {}
                gv.api = {}

                --- @return integer
                function gv.api.fetch() end

                local a = gv.api.fe<??>tch()
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) fetch() -> integer\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// An unannotated parameter takes its shape from the fields the body reads, writes and calls.
    #[gtest]
    fn test_hover_unannotated_param_usage_shape() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local function f(<??>a, b)
                    a.x = 1
                    a.y = {}
                    a.z = "s"
                    a.w = function() end
                    a.flag = true
                    b.other = 2
                    return a.read, a:call(), a["quoted"]
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) a: { [\"quoted\"]: unknown, call: unknown, flag: boolean, read: unknown, w: function, x: integer, y: table, z: string }\n```"
                    .to_string(),
            },
        ));
        Ok(())
    }

    /// Comparing a member of an untyped value must not narrow the value to `never`.
    #[gtest]
    fn test_hover_unknown_not_narrowed_to_never_by_member_comparison() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local function f(o)
                    if o.kind == "x" then
                        return o<??>.name
                    end
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) o: { kind: unknown, name: unknown }\n```".to_string(),
            },
        ));
        check!(ws.check_hover(
            r#"
                local function g(o)
                    if o.enabled then
                        return o<??>.name
                    end
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) o: { enabled: unknown, name: unknown }\n```"
                    .to_string(),
            },
        ));
        Ok(())
    }

    /// The shape is shown at every use of the parameter, not only at its declaration.
    #[gtest]
    fn test_hover_unannotated_param_usage_shape_at_use() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local function f(a)
                    local kind = a.kind
                    return a<??>.name
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) a: { kind: unknown, name: unknown }\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A local whose initialiser is unknown gets the same usage shape.
    #[gtest]
    fn test_hover_unknown_local_usage_shape() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local t<??> = make()
                t.code = 0
                print(t.stderr)
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal t: { code: integer, stderr: unknown }\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// Only direct field uses count; a nested access contributes its first key.
    #[gtest]
    fn test_hover_usage_shape_ignores_nested_and_unused() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local function f(<??>c)
                    c.d.e = 3
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) c: { d: unknown }\n```".to_string(),
            },
        ));
        check!(ws.check_hover(
            r#"
                local function g(<??>u)
                    return 1
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) u: unknown\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// An annotated parameter keeps its declared type; no shape is synthesised.
    #[gtest]
    fn test_hover_annotated_param_keeps_declared_type() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@param a any
                local function f(<??>a)
                    return a.x
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) a: any\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A parameter declared as a class lists the class members, like a local of that class does.
    #[gtest]
    fn test_hover_class_param_expands_members() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Entry
                ---@field a integer
                ---@field b string

                ---@param e Entry
                local function f(<??>e)
                    return e.a
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) e: Entry {\n    a: integer,\n    b: string,\n}\n```"
                    .to_string(),
            },
        ));
        check!(ws.check_hover(
            r#"
                ---@param e Entry
                local function g(e)
                    return <??>e.a
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) e: Entry {\n    a: integer,\n    b: string,\n}\n```"
                    .to_string(),
            },
        ));
        Ok(())
    }

    /// Hovering an assignment target shows the slot of a multi-return call it receives, not the whole tuple.
    #[gtest]
    fn test_hover_assignment_target_takes_its_multi_return_slot() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local function two()
                    return 1, "x"
                end

                local a
                <??>a = two()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal a: number\n```".to_string(),
            },
        ));
        check!(ws.check_hover(
            r#"
                local function two()
                    return 1, "x"
                end

                local b, c
                b, <??>c = two()
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal c: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A field whose type is a class lists the class members, like a local of that class does.
    #[gtest]
    fn test_hover_class_typed_field_expands_members() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class GitObj
                ---@field file string
                ---@field n integer

                ---@class Cache
                ---@field git_obj GitObj
                local Cache = {}

                function Cache:use()
                    return self.git<??>_obj.file
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) Cache.git_obj: GitObj {\n    file: string,\n    n: integer,\n}\n```"
                    .to_string(),
            },
        ));
        Ok(())
    }

    /// A nullable class local lists the members of the class, keeping the `?` on the header.
    #[gtest]
    fn test_hover_nullable_class_local_expands_members() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Entry
                ---@field a integer

                ---@type Entry?
                local <??>o = nil
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal o: Entry? {\n    a: integer,\n}\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// The `---@version` branches that do not apply to the runtime are not the signature shown.
    #[gtest]
    fn test_hover_stdlib_version_overload_picks_runtime_version() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        for source in [
            "local a = coroutine.run<??>ning",
            "local a = coroutine.run<??>ning()",
        ] {
            let (content, position) = ProviderVirtualWorkspace::handle_file_content(source)?;
            let file_id = ws.def(&content);
            let hover = crate::handlers::hover::hover(&ws.analysis, file_id, position)
                .ok_or("no hover")
                .or_fail()?;
            let hover_text = format!("{hover:?}");
            verify_that!(
                hover_text,
                contains_substring("function coroutine.running() -> thread, boolean")
            )?;
            verify_that!(hover_text, not(contains_substring("thread?")))?;
        }
        Ok(())
    }

    /// A `self` field that holds an empty or unknown value takes its shape from how the methods use it.
    #[gtest]
    fn test_hover_self_field_usage_shape() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local M = {}

                function M:init()
                    self.params = {}
                    self.dao = nil
                end

                function M:run()
                    self.params.acls = 1
                    local g = self.params.group
                    self.dao:page(1)
                    return self.pa<??>rams
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) self.params: { acls: integer, group: unknown }\n```"
                    .to_string(),
            },
        ));
        check!(ws.check_hover(
            r#"
                local M = {}

                function M:init()
                    self.dao = nil
                end

                function M:run()
                    return self.d<??>ao:page(1)
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) self.dao: { page: unknown }\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// The field of a `require(...)` call is hovered as the module member, not as a bare `function`.
    #[gtest]
    fn test_hover_field_of_require_call() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "shuffle_mod.lua",
            r#"
                return {
                    shuffle = function(t, seed)
                        return t
                    end,
                }
            "#,
        );
        check!(ws.check_hover(
            r#"
                local shuffle = require("shuffle_mod").shu<??>ffle
            "#,
            VirtualHoverResult {
                value: "```lua\nfunction shuffle(t: any, seed: any) -> unknown\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// Lua coerces a number to a string for the string library, so the subject accepts both.
    #[gtest]
    fn test_hover_string_library_accepts_number_subject() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        for name in ["lower", "upper", "format", "len", "reverse", "rep", "byte"] {
            let block = format!("local x = string.{name}<??>");
            let (content, position) = ProviderVirtualWorkspace::handle_file_content(&block)?;
            let file_id = ws.def(&content);
            let hover = crate::handlers::hover::hover(&ws.analysis, file_id, position)
                .ok_or("no hover")
                .or_fail()?;
            let hover_text = format!("{hover:?}");
            verify_that!(hover_text, contains_substring("(string|number)"))?;
        }
        Ok(())
    }

    /// `self:m()` inside a method of a class built by a factory function hovers the method.
    #[gtest]
    fn test_hover_self_method_on_factory_class() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local mt = {}
                local function class()
                  local c = setmetatable({}, mt)
                  c.__index = c
                  return c
                end

                local Lin = class()

                function Lin:a(node, x)
                  return 1
                end

                function Lin:b(node)
                  self:<??>a(node)
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(method) Lin:a(node: any, x: any) -> number\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A mistyped argument still yields the return type of a global function's only signature.
    #[gtest]
    fn test_hover_global_call_with_mismatched_arg_keeps_return() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@param i integer
                ---@return string
                function gf(i) return "" end

                local s<??> = gf(1.5)
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal s: string\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A call to a module function declared with `---@overload` lists the declared signature and
    /// the overload, whichever one the call matches.
    #[gtest]
    fn test_hover_member_call_lists_declared_overloads() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local M = {}
                ---Doc here.
                ---@overload fun(list: table, value: any)
                ---@param list table
                ---@param pos integer
                ---@param value any
                function M.ins(list, pos, value) end

                M.<??>ins({}, 1)
            "#,
            VirtualHoverResult {
                value: "```lua\nfunction M.ins(list: table, pos: integer, value: any) (+1 overloads)\n```\n\n---\n\nDoc here.\n\n---\n\n```lua\nfunction M.ins(list: table, value: any)\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// A local holding `require("mod")` prints the module's table shape, not `M|{...}` with the
    /// exported local's name.
    #[gtest]
    fn test_hover_required_module_local_prints_shape_only() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "mod.lua",
            r#"
                local M = {}
                M.n = 1
                return M
            "#,
        );
        check!(ws.check_hover(
            r#"
                local m<??>od = require("mod")
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal mod: { n: number }\n```".to_string(),
            },
        ));
        Ok(())
    }

    #[gtest]
    fn test_hover_std_table_insert_call_lists_overloads() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        let (content, position) =
            ProviderVirtualWorkspace::handle_file_content("table.<??>insert({}, 1)")?;
        let file_id = ws.def(&content);
        let hover = crate::handlers::hover::hover(&ws.analysis, file_id, position)
            .ok_or("no hover")
            .or_fail()?;
        let hover_text = format!("{hover:?}");
        verify_that!(
            hover_text,
            contains_substring("table.insert(list: table, pos: integer, value: any) (+1 overloads)")
        )?;
        verify_that!(hover_text, contains_substring("table.insert(list: table, value: any)"))?;
        Ok(())
    }

    /// io.open hovers with LuaLS's name for the failure message return.
    #[gtest]
    fn test_hover_io_open_names_errmsg() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        let (content, position) =
            ProviderVirtualWorkspace::handle_file_content("local f, e = io.<??>open('x')")?;
        let file_id = ws.def(&content);
        let hover = crate::handlers::hover::hover(&ws.analysis, file_id, position)
            .ok_or("no hover")
            .or_fail()?;
        let hover_text = format!("{hover:?}");
        verify_that!(hover_text, contains_substring("errmsg: string?"))?;
        Ok(())
    }
}
