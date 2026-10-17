use crate::{LuaType, VirtualWorkspace};

/// Each field is assigned from the previous one, so typing the last asks for a recursion as deep
/// as the chain is long.
fn field_chain(links: usize) -> String {
    let mut source = String::from("Chain = {}\nChain.f0 = 1\n");
    for link in 1..links {
        source.push_str(&format!("Chain.f{link} = Chain.f{}\n", link - 1));
    }
    source
}

#[test]
fn a_long_field_chain_does_not_overflow_the_stack() {
    const LINKS: usize = 4000;
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(&field_chain(LINKS));
    let ty = ws.expr_ty(&format!("Chain.f{}", LINKS - 1));
    // The end of a chain that long may be unknown; reaching this line is the point.
    assert!(matches!(
        ty,
        LuaType::Integer | LuaType::IntegerConst(_) | LuaType::Unknown
    ));
}
