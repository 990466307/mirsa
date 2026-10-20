#![feature(rustc_private)]
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::combined::CombinedState;
use mirsa_domains::{
    allocation::AllocationState,
    interval::{Interval, IntervalState},
    nullptr::{NullPtr, NullPtrState},
};
use mirsa_framework::{access_path::AccessPath, forward::DomainState};
use mirsa_relations::{symbolic::SymbolicExpr, value_expr::ValueExpr};
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::{mir::*, ty::TyCtxt};

fn entry<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> CombinedState<'tcx> {
    let places: Vec<_> = body.local_decls.indices().map(Place::from).collect();
    CombinedState::new_with_debug(
        IntervalState::new_bot_state(tcx, &body.local_decls, &places, &[], body.arg_count, false),
        NullPtrState::new_bot_state(&[Place::from(Local::from_usize(4))], body.arg_count, false),
        AllocationState::new_entry(tcx, body, &places, false),
        false,
        false,
    )
}
fn set(state: &mut CombinedState<'_>, local: usize, lo: i128, hi: i128) {
    state.interval.set_interval_resolved(
        &state.symbolic,
        Place::from(Local::from_usize(local)),
        Interval::new(lo, hi),
    );
}
fn value(state: &CombinedState<'_>, local: usize) -> Interval {
    state
        .interval
        .tracked_interval_resolved(&state.symbolic, &Place::from(Local::from_usize(local)))
        .unwrap()
}
fn sum<'tcx>(left: Place<'tcx>, right: Place<'tcx>) -> SymbolicExpr<'tcx> {
    SymbolicExpr::Value(ValueExpr::Binary {
        op: BinOp::Add,
        left: Operand::Copy(left),
        right: Operand::Copy(right),
    })
}
// Cloning drops memoization. Compare each cached request with a fresh reduction
// from exactly the same semantic state, including the amount of refinement.
fn compare<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &mut CombinedState<'tcx>,
    place: Place<'tcx>,
) {
    let mut fresh = state.clone();
    assert_eq!(
        state.reduce_place(tcx, &body.local_decls, place),
        fresh.reduce_place(tcx, &body.local_decls, place)
    );
    assert_eq!(*state, fresh);
}
struct Checks;
impl Callbacks for Checks {
    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        let id = tcx
            .mir_keys(())
            .iter()
            .find(|id| tcx.item_name(id.to_def_id()).as_str() == "inputs")
            .unwrap();
        let body = tcx.optimized_mir(id.to_def_id());
        let [a, b, c, p] = [1, 2, 3, 4].map(|i| Place::from(Local::from_usize(i)));
        let mut state = entry(tcx, body);
        set(&mut state, 1, 0, 10);
        set(&mut state, 2, 0, 100);
        set(&mut state, 3, 1, 1);
        state.symbolic.set_place_expr(b, sum(a, c));
        for _ in 0..3 {
            compare(tcx, body, &mut state, b);
        }
        assert_eq!(value(&state, 2), Interval::new(1, 11));
        set(&mut state, 1, 5, 5);
        compare(tcx, body, &mut state, b);
        assert_eq!(value(&state, 2), Interval::new(6, 6));
        // A symbolic write must invalidate the previous stable result too.
        state
            .symbolic
            .set_place_expr(b, SymbolicExpr::Value(ValueExpr::Use(Operand::Copy(a))));
        set(&mut state, 2, 0, 100);
        compare(tcx, body, &mut state, b);
        assert_eq!(value(&state, 2), Interval::new(5, 5));
        let mut joined = CombinedState::join(&state, &entry(tcx, body));
        compare(tcx, body, &mut joined, b);
        let mut widened = CombinedState::widen(&state, &joined);
        compare(tcx, body, &mut widened, b);

        // Cross-domain refinement after an earlier stable pointer query.
        let mut state = entry(tcx, body);
        state.symbolic.set_place_expr(
            a,
            SymbolicExpr::Address {
                pointer: Operand::Copy(p),
            },
        );
        set(&mut state, 1, 0, i64::MAX as i128);
        for _ in 0..3 {
            compare(tcx, body, &mut state, a);
        }
        state
            .nullptr
            .set_path(AccessPath::from_place(p).unwrap(), NullPtr::Null);
        compare(tcx, body, &mut state, a);
        assert_eq!(value(&state, 1), Interval::new(0, 0));

        // Bounded reductions of cyclic relations need further calls: a request
        // that changed facts must never be mistaken for a fixed point.
        let mut state = entry(tcx, body);
        set(&mut state, 1, 0, 100);
        set(&mut state, 2, 0, 100);
        set(&mut state, 3, 1, 1);
        state.symbolic.set_place_expr(a, sum(b, c));
        state.symbolic.set_place_expr(b, sum(a, c));
        compare(tcx, body, &mut state, a);
        let before = value(&state, 1).low;
        compare(tcx, body, &mut state, a);
        assert!(value(&state, 1).low > before);
        Compilation::Stop
    }
}
#[test]
fn cached_reduction_preserves_refinement_and_invalidation() {
    let source = std::env::temp_dir().join(format!("mirsa-reduce-cache-{}.rs", std::process::id()));
    std::fs::write(
        &source,
        "pub fn inputs(a:i64,b:i64,c:i64,p:*const u8)->i64 { a }",
    )
    .unwrap();
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            source.to_string_lossy().into_owned(),
            "--crate-name=reduce_cache".into(),
            "--crate-type=lib".into(),
            "--emit=metadata".into(),
            "-Zmir-opt-level=0".into(),
            "-Awarnings".into(),
        ],
        &mut Checks,
    );
    std::fs::remove_file(source).unwrap();
}
