#[cfg(test)]
mod test {
    use crate::VirtualWorkspace;

    #[test]
    fn test_call_omitting_optional_param_of_literal_union() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let ty = ws.expr_ty("io.popen('ls')");
        assert_eq!(ws.humanize_type(ty), "file");
    }

    #[test]
    fn test_call_omitting_optional_param_of_nullable_union_with_class() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let ty = ws.expr_ty("io.output()");
        assert_eq!(ws.humanize_type(ty), "file");
    }
}
