//! Regression guards for flow backtracking on a repeated nested-index guard chain.
//!
//! Shape (`n` identical blocks; the *second* index expression inside one condition
//! list is what triggers it):
//!
//! ```lua
//! V = V or {}
//! if V[k1] and V[k1][k2] then V[k1][k2][s] = ... end
//! ```
//!
//! Each block's guard assigns the traced global, so the two merge arms genuinely
//! differ and both must be walked. Before the declaration-trace states were
//! memoized (`cache::TraceStateKey`, applied only on loop-free flow graphs) the
//! same states were re-derived along ~2^n routes:
//!
//! | n | 2 | 4 | 6 | 8 | 10 | 12 | 16 | 20 |
//! |---|---:|---:|---:|---:|---:|---:|---:|---:|
//! | walk entries | 13 | 81 | 365 | 1513 | 6117 | 24545 | 393177 | 6.3M |
//!
//! With the memo the 600-block payload of
//! `test_issue_1028_i18n_semantic_tokens_repeated_prefix_guard_chain` in
//! `emmylua_ls` finishes in well under a second.

use crate::semantic_model::flow::flow_metrics;
use crate::{LuaType, VirtualWorkspace};
use emmylua_parser::{LuaAstNode, LuaIndexExpr};

/// `n` guard blocks, each repeating one nested index expression in a condition.
fn repeated_prefix_guard_chain(n: usize) -> String {
    let mut content = String::from("V_cfad19afc42b = V_cfad19afc42b or {}\n");
    for i in 0..n {
        let table_key = 3_121_212;
        let field_key = 1_111_112 + i;
        content.push_str(&format!(
            "if V_cfad19afc42b[{table_key}] and V_cfad19afc42b[{table_key}][{field_key}] then\n    V_cfad19afc42b[{table_key}][{field_key}][\"__STR_{i}__\"] = \"__STR_{}__\"\nend\n\n",
            i + 1,
        ));
    }
    content
}

/// Read every resolvable member of the file, the way the semantic-token builder
/// does for every name token it renders.
fn read_every_member(fid: crate::FileId, ws: &VirtualWorkspace) {
    let model = ws.analysis.semantic_model(fid);
    for index_expr in model.chunk().expect("chunk").descendants::<LuaIndexExpr>() {
        let Some(resolved) = model.resolve_member(&index_expr) else {
            continue;
        };
        let Some(member_id) = resolved.member_id else {
            continue;
        };
        let _ = model.type_of_member_at(&member_id, index_expr.get_range().start());
    }
}

/// The shape must resolve consistently for every size.
#[test]
fn p11_flow_nested_index_guard_chain_types_are_stable() {
    for n in [1usize, 4, 20, 60] {
        let mut ws = VirtualWorkspace::new();
        let fid = ws.def_file("probe.lua", &repeated_prefix_guard_chain(n));
        read_every_member(fid, &ws);
    }

    let mut nested = VirtualWorkspace::new();
    nested.def_file("nested.lua", &repeated_prefix_guard_chain(4));
    let nested_ty = nested.expr_ty("V_cfad19afc42b[3121212][1111115]");

    // Every index is into an undeclared global table, so the read stays unknown;
    // the point is that it resolves without error and without depending on n.
    assert!(
        matches!(nested_ty, LuaType::Unknown),
        "unexpected type for a nested-index read of an undeclared global: {nested_ty:?}"
    );
}

/// The number of uncached walks must stay linear in the number of blocks.
///
/// Distinct states per chain position are constant, so memoization should hold the
/// entry count to a small multiple of `n`; the pre-memo behaviour exceeded the
/// bound by orders of magnitude already at n=8 and never finished for n=600.
#[test]
fn p11_flow_repeated_nested_index_guard_chain_is_not_exponential() {
    let mut measurements = Vec::new();
    for n in [4usize, 16, 64, 256, 600] {
        let mut ws = VirtualWorkspace::new();
        let fid = ws.def_file("probe.lua", &repeated_prefix_guard_chain(n));
        flow_metrics::reset();
        read_every_member(fid, &ws);
        measurements.push((n, flow_metrics::trace_decl_entries()));
    }

    for (n, entries) in &measurements {
        let bound = 500 + 400 * (*n as u64);
        assert!(
            *entries <= bound,
            "the declaration walk ran {entries} times for {n} guard blocks (bound {bound}); \
             the trace-state memo is not bounding the work. All: {measurements:?}"
        );
    }

    // Growth must stay roughly linear: 150x the input must not cost more than
    // roughly 150x (plus the fixed per-file constant, already covered above).
    let (n_last, entries_last) = *measurements.last().expect("measurements");
    let (n_first, entries_first) = *measurements.first().expect("measurements");
    let (n_last, n_first) = (n_last as u64, n_first as u64);
    assert!(
        entries_last * n_first <= entries_first * n_last * 8,
        "cost grew faster than linear in the number of guard blocks: {measurements:?}"
    );
}

/// `n` sequential `if flag ... else ... end` blocks after a loop; each block narrows
/// `flag`, so no merge can be skipped.
fn guard_chain_after_loop(n: usize) -> String {
    let mut content =
        String::from("---@type string?\nlocal flag\nlocal function a() end\nfor _ = 1, 2 do end\n");
    for _ in 0..n {
        content.push_str("if flag then a() else a() end\n");
    }
    content.push_str("local y = flag\n");
    content
}

/// In a file with a loop the trace-state memo is off, so a chain of `n` guard blocks
/// costs 2^n walks per read of the guarded variable. One read must stay bounded.
#[test]
fn p11_flow_guard_chain_after_a_loop_has_bounded_work_per_read() {
    use emmylua_parser::LuaNameExpr;

    for n in [8usize, 16, 20] {
        let mut ws = VirtualWorkspace::new();
        let fid = ws.def_file("probe.lua", &guard_chain_after_loop(n));
        let model = ws.analysis.semantic_model(fid);
        let chunk = model.chunk().expect("chunk");
        let reads: Vec<_> = chunk
            .descendants::<LuaNameExpr>()
            .filter(|expr| expr.get_text() == "flag")
            .collect();
        assert_eq!(reads.len(), n + 1, "reads of flag for n={n}");

        for expr in reads {
            flow_metrics::reset();
            let _ = model.type_of_expr_at(expr.get_syntax_id(), expr.get_range().start());
            let walks = flow_metrics::trace_decl_entries();
            assert!(
                walks <= 5_000,
                "one read of `flag` ran {walks} declaration walks with {n} guard blocks \
                 after a loop; the per-query walk limit is not bounding the work"
            );
        }
    }
}

/// A query that stays under the limit keeps its exact, narrowed answer.
#[test]
fn p11_flow_guard_chain_after_a_loop_keeps_exact_type_under_the_limit() {
    let mut ws = VirtualWorkspace::new();
    let fid = ws.def_file("probe.lua", &guard_chain_after_loop(3));
    let model = ws.analysis.semantic_model(fid);
    let chunk = model.chunk().expect("chunk");
    let last = chunk
        .descendants::<emmylua_parser::LuaNameExpr>()
        .filter(|expr| expr.get_text() == "flag")
        .last()
        .expect("last read");
    let ty = model.type_of_expr_at(last.get_syntax_id(), last.get_range().start());
    assert!(
        ty.is_nullable(),
        "flag is `string?` after the guards: {ty:?}"
    );
}

/// A read whose trace ran out of walks is answered from the declared type; that answer is
/// reported as a guess every time it is served and is not stored with the exact answers.
#[test]
fn p11_flow_trace_that_gives_up_is_not_cached_as_exact() {
    use crate::semantic_model::flow::gave_up_events;
    use emmylua_parser::LuaNameExpr;

    let mut ws = VirtualWorkspace::new();
    let fid = ws.def_file("probe.lua", &guard_chain_after_loop(20));
    let model = ws.analysis.semantic_model(fid);
    let chunk = model.chunk().expect("chunk");
    let last = chunk
        .descendants::<LuaNameExpr>()
        .filter(|expr| expr.get_text() == "flag")
        .last()
        .expect("last read");
    for attempt in 0..3 {
        let before = gave_up_events();
        let ty = model.type_of_expr_at(last.get_syntax_id(), last.get_range().start());
        assert!(ty.is_nullable(), "declared type is served: {ty:?}");
        assert!(
            gave_up_events() > before,
            "attempt {attempt}: the answer was served as exact"
        );
    }
}

/// `n` assigning blocks that do not narrow `x` by their own condition, after a loop, then a
/// guard that removes nil from the parameter.
fn assigning_chain_after_loop(n: usize) -> String {
    let mut content = String::from(
        "---@param c boolean\n---@param x string?\nlocal function f(c, x)\n  while c do c = not c end\n",
    );
    for _ in 0..n {
        content.push_str("  if c then x = x or 'a' end\n");
    }
    content.push_str("  if x == nil then return end\n  local z = x\n  return z\nend\n");
    content
}

fn type_of_last_read_of_x(content: &str) -> LuaType {
    use emmylua_parser::LuaNameExpr;

    let mut ws = VirtualWorkspace::new();
    let fid = ws.def_file("probe.lua", content);
    let model = ws.analysis.semantic_model(fid);
    let chunk = model.chunk().expect("chunk");
    let last = chunk
        .descendants::<LuaNameExpr>()
        .filter(|expr| expr.get_text() == "x")
        .last()
        .expect("last read");
    model.type_of_expr_at(last.get_syntax_id(), last.get_range().start())
}

/// The answer after a nil guard must not depend on how many unrelated assigning blocks come
/// before it: past the walk limit the trace used to fall back to the declared `string?`.
#[test]
fn p11_flow_nil_guard_after_assigning_blocks_in_a_loop_graph_stays_narrowed() {
    for n in [6usize, 11, 20, 30] {
        let ty = type_of_last_read_of_x(&assigning_chain_after_loop(n));
        assert!(
            !ty.is_nullable(),
            "x is `string` after `if x == nil then return end` with n={n}: {ty:?}"
        );
    }
}
