#[cfg(test)]
mod tests {
    use crate::handlers::call_hierarchy::{incoming_calls, prepare_call_hierarchy};
    use crate::handlers::test_lib::{ProviderVirtualWorkspace, check};
    use googletest::prelude::*;

    /// `(caller name, call-site lines)` for every incoming call to the function under `<??>`.
    fn incoming(ws: &mut ProviderVirtualWorkspace, block: &str) -> Result<Vec<(String, Vec<u32>)>> {
        let (content, position) = check!(ProviderVirtualWorkspace::handle_file_content(block));
        let file_id = ws.def(&content);
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
}
