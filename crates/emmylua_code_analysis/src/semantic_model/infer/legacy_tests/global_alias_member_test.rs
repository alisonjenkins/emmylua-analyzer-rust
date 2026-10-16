#[cfg(test)]
mod test {
    use crate::VirtualWorkspace;

    fn type_of_t(source: &str) -> String {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
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
    fn test_call_through_local_alias_of_class_table_global() {
        assert_eq!(type_of_t("local io = io\nlocal t = io.open('x')"), "file?");
    }

    #[test]
    fn test_call_through_differently_named_alias_of_class_table_global() {
        assert_eq!(type_of_t("local o = io\nlocal t = o.tmpfile()"), "file");
    }
}
