//! Bookkeeping that keeps closure-return inference finite and cheap across files.
//!
//! A closure's return type is inferred from its body. When the body calls a function of another
//! file, that file gets its own `SemanticModel` (its syntax tree and scope belong to it), and the
//! new model starts with empty per-model state. Two things must therefore live outside any one
//! model: the stack of closures being inferred (so a cycle that crosses files is cut at the first
//! repeat instead of unrolling until a depth limit) and the finished results (so a function reached
//! through many call paths is inferred once, not once per path).
//!
//! A result may only be kept when it does not depend on what the caller was in the middle of.
//! Cutting a cycle makes a result depend on the closures above the cut, and a depth limit makes it
//! depend on the depth it was reached at; [`ClosureReturnShared::end`] tracks both.

use std::cell::{Cell, RefCell};

use hashbrown::HashMap;

use crate::{FileId, LuaType};
use emmylua_parser::LuaSyntaxId;

/// A closure in any file.
pub(crate) type ClosureKey = (FileId, LuaSyntaxId);

/// Shared by a model and every foreign-file model it creates (see `SemanticModel::model_for`).
pub(crate) struct ClosureReturnShared {
    /// Finished results that do not depend on the caller.
    memo: RefCell<HashMap<ClosureKey, LuaType>>,
    /// Results that a cycle cut or a depth limit shaped. They are reused until the outermost
    /// inference ends, so a function in a cycle of calls is inferred once per top-level query
    /// instead of once per path through the cycle; the next query starts clean.
    within_root: RefCell<HashMap<ClosureKey, LuaType>>,
    /// Closures whose return type is being inferred, outermost first.
    stack: RefCell<Vec<ClosureKey>>,
    /// Lowest stack index a cycle cut has pointed at since the innermost open frame began.
    lowest_cut: Cell<usize>,
}

thread_local! {
    /// Depth-limit cuts so far on this thread. A result computed across one depends on its depth.
    static DEPTH_CUTS: Cell<u32> = const { Cell::new(0) };
}

/// Records that a depth limit truncated the work in progress.
pub(crate) fn note_depth_cut() {
    DEPTH_CUTS.with(|cuts| cuts.set(cuts.get().wrapping_add(1)));
}

/// Stack bytes a recursive query may use below the shallowest guarded frame seen on this thread.
///
/// Worker threads have 2 MiB; the rest is left to the frames between two guard checks and to
/// whatever called into the analysis.
const STACK_BUDGET: usize = 1_200_000;

thread_local! {
    static STACK_TOP: Cell<usize> = const { Cell::new(0) };
}

/// True when a recursive query is deep enough that going further risks a stack overflow. The
/// caller answers unknown instead of recursing, which turns a crash into a missing type.
///
/// How deep a query goes depends on where the thread's stack was when it started, so a result
/// computed across a cut like this is never kept as a closure's return type.
pub(crate) fn stack_budget_exhausted() -> bool {
    let marker = 0u8;
    let here = std::ptr::addr_of!(marker) as usize;
    let exhausted = STACK_TOP.with(|top| {
        // The stack grows down: the highest address seen is the shallowest frame.
        if here > top.get() {
            top.set(here);
        }
        top.get() - here > STACK_BUDGET
    });
    if exhausted {
        note_stack_cut();
    }
    exhausted
}

fn depth_cuts() -> u32 {
    DEPTH_CUTS.with(|cuts| cuts.get())
}

thread_local! {
    /// Stack-budget cuts so far on this thread; see [`stack_budget_exhausted`].
    static STACK_CUTS: Cell<u32> = const { Cell::new(0) };
}

fn note_stack_cut() {
    STACK_CUTS.with(|cuts| cuts.set(cuts.get().wrapping_add(1)));
}

fn stack_cuts() -> u32 {
    STACK_CUTS.with(|cuts| cuts.get())
}

impl Default for ClosureReturnShared {
    fn default() -> Self {
        Self {
            memo: RefCell::new(HashMap::new()),
            within_root: RefCell::new(HashMap::new()),
            stack: RefCell::new(Vec::new()),
            lowest_cut: Cell::new(usize::MAX),
        }
    }
}

/// An open inference; hand it back to [`ClosureReturnShared::end`].
pub(crate) struct ClosureFrame {
    index: usize,
    saved_cut: usize,
    depth_cuts_at_start: u32,
    stack_cuts_at_start: u32,
}

impl ClosureReturnShared {
    pub(crate) fn memoized(&self, key: &ClosureKey) -> Option<LuaType> {
        let kept = self.memo.borrow().get(key).cloned();
        kept.or_else(|| self.within_root.borrow().get(key).cloned())
    }

    /// Whether `key` is being inferred further up. The caller then answers unknown, so every
    /// frame above the repeat is now context dependent.
    pub(crate) fn is_open(&self, key: &ClosureKey) -> bool {
        let stack = self.stack.borrow();
        let Some(position) = stack.iter().position(|open| open == key) else {
            return false;
        };
        self.lowest_cut.set(self.lowest_cut.get().min(position));
        true
    }

    /// Opens an inference of `key`; `None` when it is already open (see [`Self::is_open`]).
    pub(crate) fn begin(&self, key: ClosureKey) -> Option<ClosureFrame> {
        if self.is_open(&key) {
            return None;
        }
        let mut stack = self.stack.borrow_mut();
        let frame = ClosureFrame {
            index: stack.len(),
            saved_cut: self.lowest_cut.get(),
            depth_cuts_at_start: depth_cuts(),
            stack_cuts_at_start: stack_cuts(),
        };
        stack.push(key);
        Some(frame)
    }

    /// Closes `frame`. With `keep`, the result is stored when nothing above the frame shaped it.
    ///
    /// `entered_at_depth_zero` is whether the caller was outside every depth-limited walk; a
    /// depth cut inside such a frame is the same every time, so that result is kept.
    pub(crate) fn end(&self, frame: ClosureFrame, keep: Option<(&LuaType, bool)>) {
        let key = self.stack.borrow_mut().pop();
        let cut_above = self.lowest_cut.get() < frame.index;
        if !cut_above {
            self.lowest_cut.set(frame.saved_cut);
        }
        if let (Some((result, entered_at_depth_zero)), Some(key)) = (keep, key) {
            let depth_dependent =
                depth_cuts() != frame.depth_cuts_at_start && !entered_at_depth_zero;
            let stack_dependent = stack_cuts() != frame.stack_cuts_at_start;
            if !cut_above && !depth_dependent && !stack_dependent {
                self.memo.borrow_mut().insert(key, result.clone());
            } else {
                self.within_root.borrow_mut().insert(key, result.clone());
            }
        }
        if self.stack.borrow().is_empty() {
            self.within_root.borrow_mut().clear();
        }
    }
}
