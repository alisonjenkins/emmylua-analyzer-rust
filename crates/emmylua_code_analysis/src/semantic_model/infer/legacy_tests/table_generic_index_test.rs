#[cfg(test)]
mod test {
    use crate::VirtualWorkspace;

    fn type_of_t(source: &str) -> String {
        let mut ws = VirtualWorkspace::new();
        let file_id = ws.def(source);
        let model = ws.analysis.semantic_model(file_id);
        let decl = model
            .decls()
            .and_then(|decls| decls.iter().find(|decl| decl.name == "t"))
            .expect("local t")
            .id
            .clone();
        let ty = model.type_of_decl(&decl).expect("type of t");
        ws.humanize_type(ty)
    }

    #[test]
    fn test_index_with_unknown_key_gives_the_value_type() {
        assert_eq!(
            type_of_t(
                "---@param m table<integer, string>\nlocal function f(m, k)\n  local t = m[k]\nend"
            ),
            "string"
        );
    }

    #[test]
    fn test_index_with_matching_literal_key_gives_the_value_type() {
        assert_eq!(
            type_of_t(
                "---@param m table<string, integer>\nlocal function f(m)\n  local t = m['x']\nend"
            ),
            "integer"
        );
        assert_eq!(
            type_of_t(
                "---@param m table<integer, string>\nlocal function f(m)\n  local t = m[1]\nend"
            ),
            "string"
        );
    }

    #[test]
    fn test_field_access_on_string_keyed_table_gives_the_value_type() {
        assert_eq!(
            type_of_t(
                "---@param m table<string, integer>\nlocal function f(m)\n  local t = m.x\nend"
            ),
            "integer"
        );
    }

    #[test]
    fn test_index_with_a_key_of_the_wrong_kind_stays_unknown() {
        assert_eq!(
            type_of_t(
                "---@param m table<string, integer>\nlocal function f(m)\n  local t = m[1]\nend"
            ),
            "unknown"
        );
    }
}
