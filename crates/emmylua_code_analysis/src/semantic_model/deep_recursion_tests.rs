use rowan::TextSize;

use crate::{LuaType, SemanticModel, VirtualWorkspace, register_thread_stack};

use super::recursion_guard::EXPR_QUERIES;

const MIB: usize = 1024 * 1024;

/// Each field is assigned from the previous one, so typing the last asks for a recursion as deep
/// as the chain is long.
fn field_chain(links: usize) -> String {
    let mut source = String::from("Chain = {}\nChain.f0 = 1\n");
    for link in 1..links {
        source.push_str(&format!("Chain.f{link} = Chain.f{}\n", link - 1));
    }
    source
}

/// Runs `body` on a thread of `stack_bytes`, which has told the guard so when `registered`.
fn on_worker_stack<T: Send + 'static>(
    stack_bytes: usize,
    registered: bool,
    body: impl FnOnce() -> T + Send + 'static,
) -> T {
    std::thread::Builder::new()
        .stack_size(stack_bytes)
        .spawn(move || {
            if registered {
                register_thread_stack(stack_bytes);
            }
            body()
        })
        .expect("spawn worker thread")
        .join()
        .expect("the query must not panic")
}

/// Uses about `bytes` of stack, then runs `body` below it.
#[inline(never)]
fn with_stack_used<T>(bytes: usize, body: impl FnOnce() -> T) -> T {
    let pad = [0u8; 16 * 1024];
    std::hint::black_box(&pad);
    if bytes <= pad.len() {
        return body();
    }
    let result = with_stack_used(bytes - pad.len(), body);
    std::hint::black_box(&pad);
    result
}

/// What a query answered: `number` (what a query that reaches the end of the chain gives),
/// an integer or unknown (a query cut short), or other.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Answer {
    Number,
    Integer,
    Unknown,
    Other,
}

fn answer_of(ty: &LuaType) -> Answer {
    match ty {
        LuaType::Number => Answer::Number,
        LuaType::Integer | LuaType::IntegerConst(_) => Answer::Integer,
        LuaType::Unknown => Answer::Unknown,
        _ => Answer::Other,
    }
}

/// A file holding the chain followed by `local <name> = Chain.f<link>` for each of `locals`.
struct ChainFile {
    ws: VirtualWorkspace,
    file: crate::FileId,
    source: String,
}

impl ChainFile {
    fn new(links: usize, locals: &[(&str, usize)]) -> Self {
        let mut source = field_chain(links);
        for (name, link) in locals {
            source.push_str(&format!("local {name} = Chain.f{link}\n"));
        }
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        Self { ws, file, source }
    }

    fn model(&self) -> SemanticModel<'_> {
        self.ws.analysis.semantic_model(self.file)
    }

    /// What `model` answers for the type of local `name`.
    fn answer(&self, model: &SemanticModel, name: &str) -> Answer {
        let at = self
            .source
            .rfind(&format!("local {name} ="))
            .expect("local")
            + "local ".len();
        let offset = TextSize::from(u32::try_from(at).expect("offset"));
        let decl = model.decl_by_offset(offset).expect("local is declared");
        answer_of(&model.type_of_decl(&decl).expect("local has a type"))
    }
}

/// Types `q`, the end of a `links` chain in the same file, and counts the expression queries.
fn type_of_chain_end_in_same_file(links: usize) -> (Answer, usize) {
    let file = ChainFile::new(links, &[("q", links - 1)]);
    let model = file.model();
    EXPR_QUERIES.with(|count| count.set(0));
    let answer = file.answer(&model, "q");
    (answer, EXPR_QUERIES.with(|count| count.get()))
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
    let (answer, queries) =
        on_worker_stack(2 * MIB, false, || type_of_chain_end_in_same_file(4000));
    assert_ne!(answer, Answer::Other);
    assert!(queries < 1_000_000, "{queries} expression queries");
}

#[test]
fn a_thousand_link_chain_is_typed_on_a_large_registered_stack() {
    let (answer, queries) =
        on_worker_stack(128 * MIB, true, || type_of_chain_end_in_same_file(1000));
    assert_eq!(answer, Answer::Number);
    // Linear in the chain, not a re-descent from every cut.
    assert!(queries < 100_000, "{queries} expression queries");
}

#[test]
fn the_order_of_queries_does_not_change_their_answers() {
    on_worker_stack(2 * MIB, true, || {
        let file = ChainFile::new(4000, &[("near", 10), ("far", 3999)]);
        let near_first = file.model();
        let near = file.answer(&near_first, "near");
        let far = file.answer(&near_first, "far");
        let near_again = file.answer(&near_first, "near");

        let far_first = file.model();
        let far_in_other_order = file.answer(&far_first, "far");
        let near_in_other_order = file.answer(&far_first, "near");

        assert_eq!(near, Answer::Number);
        assert_eq!(near_again, near);
        assert_eq!(near_in_other_order, near);
        assert_eq!(far_in_other_order, far);
    });
}

#[test]
fn a_query_started_deep_on_a_registered_stack_does_not_overflow() {
    let (answer, _) = on_worker_stack(2 * MIB, true, || {
        with_stack_used(1_000_000, || type_of_chain_end_in_same_file(4000))
    });
    assert_ne!(answer, Answer::Other);
}
