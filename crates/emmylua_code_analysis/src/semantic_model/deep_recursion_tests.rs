use rowan::TextSize;

use crate::{LuaType, VirtualWorkspace};

use super::recursion_guard::EXPR_QUERIES;

/// Each field is assigned from the previous one, so typing the last asks for a recursion as deep
/// as the chain is long.
fn field_chain(links: usize) -> String {
    let mut source = String::from("Chain = {}\nChain.f0 = 1\n");
    for link in 1..links {
        source.push_str(&format!("Chain.f{link} = Chain.f{}\n", link - 1));
    }
    source
}

/// Runs `body` on a thread with the 2 MiB stack of a language-server worker.
fn on_worker_stack<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(body)
        .expect("spawn worker thread")
        .join()
        .expect("the query must not panic")
}

/// Whether a type is what the chain assigns (an integer) rather than unknown.
fn is_integer(ty: &LuaType) -> bool {
    matches!(ty, LuaType::Integer | LuaType::IntegerConst(_))
}

/// Types `q` in a file that holds the chain and `local q = Chain.f<last>`: whether it is an
/// integer, whether it is unknown, and how many expression queries that took.
fn type_of_chain_end_in_same_file(links: usize) -> (bool, bool, usize) {
    let mut ws = VirtualWorkspace::new();
    let source = format!("{}local q = Chain.f{}\n", field_chain(links), links - 1);
    let q_offset = TextSize::from(u32::try_from(source.rfind("q =").expect("q")).expect("size"));
    let file = ws.def(&source);
    let model = ws.analysis.semantic_model(file);
    let decl = model.decl_by_offset(q_offset).expect("q must be declared");
    EXPR_QUERIES.with(|count| count.set(0));
    let ty = model.type_of_decl(&decl).expect("q must have a type");
    let queries = EXPR_QUERIES.with(|count| count.get());
    (is_integer(&ty), matches!(ty, LuaType::Unknown), queries)
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

#[test]
fn a_chain_in_the_same_file_does_not_overflow_a_worker_stack() {
    let (integer, unknown, queries) = on_worker_stack(|| type_of_chain_end_in_same_file(4000));
    assert!(integer || unknown);
    assert!(queries < 1_000_000, "{queries} expression queries");
}
