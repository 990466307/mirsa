#![feature(rustc_private)]
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::{
    interprocedural::AnalysisSession,
    query::{AnalysisView, Bindings, Output, QueryConfig},
};
use mirsa_framework::forward::PathForwardAnalysisConfig;
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::mir::{Local, Location, Place, TerminatorKind};

struct Check;
impl Callbacks for Check {
    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: rustc_middle::ty::TyCtxt<'tcx>,
    ) -> Compilation {
        let id = tcx
            .mir_keys(())
            .iter()
            .find(|id| tcx.item_name(id.to_def_id()).as_str() == "example")
            .unwrap();
        let body = tcx.optimized_mir(id.to_def_id());
        let result = AnalysisSession::new(PathForwardAnalysisConfig {
            max_paths: 8,
            widen_after_iterations: Some(8),
            max_duration: Some(std::time::Duration::from_secs(30)),
        })
        .analyze(tcx, body);
        assert!(result.completed);
        let (block, data) = body
            .basic_blocks
            .iter_enumerated()
            .find(|(_, data)| matches!(data.terminator().kind, TerminatorKind::Call { .. }))
            .unwrap();
        let location = Location {
            block,
            statement_index: data.statements.len(),
        };
        let bindings = Bindings::new()
            .value("x", Place::from(Local::from_usize(1)))
            .location("call", location);
        let view = AnalysisView::new(tcx, body, &result).with_config(QueryConfig {
            total_timeout: std::time::Duration::ZERO,
            ..Default::default()
        });
        let report = view.query("facts($x) at 'call", &bindings).unwrap();
        let Output::Facts(facts) = &report.output else {
            panic!()
        };
        use mirsa_analysis::query::{BinaryOperator, Expression, FactValue, SymbolicFact};
        let definitions: Vec<_> = facts
            .symbolic_info
            .iter()
            .filter_map(|fact| match fact {
                SymbolicFact::Definition {
                    expression: Expression::Binary { operator, left, .. },
                    ..
                } if matches!(operator, BinaryOperator::Div | BinaryOperator::BitXor) => {
                    Some((operator, left.as_ref()))
                }
                _ => None,
            })
            .collect();
        // Even with zero reduction budget, direct symbolic expressions and native bounds survive.
        assert!(definitions.len() >= 2, "{facts:#?}");
        assert!(
            definitions
                .iter()
                .any(|(op, _)| **op == BinaryOperator::Div)
        );
        assert!(
            facts
                .symbolic_info
                .iter()
                .any(|f| matches!(f, SymbolicFact::Condition { .. }))
        );
        assert!(
            facts
                .abstract_info
                .iter()
                .any(|f| f.subject == Expression::variable("$x")
                    && f.value
                        == FactValue::Integer {
                            low: "0".into(),
                            high: "99".into()
                        })
        );
        let members = facts
            .symbolic_info
            .iter()
            .find_map(|f| match f {
                SymbolicFact::SameValue { members }
                    if members.contains(&Expression::variable("$x")) =>
                {
                    Some(members)
                }
                _ => None,
            })
            .unwrap();
        assert!(definitions.iter().all(|(_, left)| {
            let Expression::Typed { expression, .. } = left else {
                return false;
            };
            members.contains(expression.as_ref())
        }));
        assert!(
            facts.symbolic_info.iter().any(|f| matches!(
                f,
                SymbolicFact::Definition {
                    expression: Expression::Cast { .. },
                    ..
                }
            )),
            "{facts:#?}"
        );
        assert!(
            facts.symbolic_info.iter().any(|f| matches!(
                f,
                SymbolicFact::Definition {
                    expression: Expression::CheckedBinary {
                        operator: BinaryOperator::AddWithOverflow,
                        ..
                    },
                    ..
                }
            )),
            "{facts:#?}"
        );
        // Uninitialized bookkeeping entries must not become impossible program constraints.
        assert!(facts.abstract_info.iter().all(|fact| match &fact.value {
            FactValue::Integer { low, high } =>
                low.parse::<i128>().unwrap() <= high.parse::<i128>().unwrap(),
            _ => true,
        }));
        let json = serde_json::to_value(facts).unwrap();
        let object = json.as_object().unwrap();
        assert_eq!(object.len(), 2);
        assert!(object.contains_key("abstract_info") && object.contains_key("symbolic_info"));
        assert!(
            json["symbolic_info"]
                .as_array()
                .unwrap()
                .iter()
                .all(|value| value.is_object())
        );
        Compilation::Stop
    }
}

#[test]
fn structured_facts_include_unbound_dependents_without_limits() {
    let dir = std::env::temp_dir().join(format!("mirsa-facts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let args = (0..2)
        .map(|i| format!("v{i}: u32"))
        .collect::<Vec<_>>()
        .join(",");
    let assignments = (0..2)
        .map(|i| {
            format!(
                "let v{i} = x {} {}u32;",
                if i == 0 { "/" } else { "^" },
                i + 2
            )
        })
        .collect::<String>();
    let values = (0..2)
        .map(|i| format!("v{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let source = dir.join("input.rs");
    std::fs::write(&source, format!("unsafe extern \"C\" {{ fn observe({args}, cast: u64, sum: u32); }} pub unsafe fn example(x:u32) {{ if x < 100 {{ {assignments} let cast = x as u64; let sum = x + 1; observe({values}, cast, sum); }} }}")).unwrap();
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            source.to_string_lossy().into_owned(),
            "--crate-type=lib".into(),
            "--edition=2021".into(),
            "--emit=metadata".into(),
            "--out-dir".into(),
            dir.to_string_lossy().into_owned(),
            "-Zmir-opt-level=0".into(),
            "-Zmir-enable-passes=-RemoveStorageMarkers".into(),
        ],
        &mut Check,
    );
    std::fs::remove_dir_all(dir).unwrap();
}
