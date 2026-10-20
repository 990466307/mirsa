use mirsa_solver::{SolveResult, UnknownReason, Verdict as SolverVerdict};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Verdict {
    T,
    F,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ProofSource {
    Abstract,
    Solver,
}

#[derive(Clone, Debug, Serialize)]
pub struct PathProof {
    pub path_id: u32,
    pub merged: bool,
    pub verdict: Verdict,
    pub definite_violation: bool,
    pub unreachable: bool,
    pub source: ProofSource,
    pub reason: Option<UnknownReason>,
    pub candidate: Option<String>,
}

impl PathProof {
    pub(super) fn new(
        path_id: u32,
        merged: bool,
        source: ProofSource,
        result: SolveResult,
    ) -> Self {
        Self {
            path_id,
            merged,
            verdict: match result.verdict {
                SolverVerdict::Proven => Verdict::T,
                _ => Verdict::F,
            },
            definite_violation: result.verdict == SolverVerdict::Refuted,
            unreachable: result.verdict == SolverVerdict::Unreachable,
            source,
            reason: result.reason,
            candidate: result.candidate,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Proof {
    pub verdict: Verdict,
    pub definite_violation: bool,
    pub unreachable: bool,
    pub reason: Option<UnknownReason>,
    pub paths: Vec<PathProof>,
}

impl Proof {
    pub(super) fn from_paths(paths: Vec<PathProof>) -> Self {
        let feasible = || paths.iter().filter(|path| !path.unreachable);
        let unreachable = feasible().next().is_none();
        let definite_violation = feasible().any(|path| path.definite_violation);
        let verdict = if !unreachable && feasible().all(|path| path.verdict == Verdict::T) {
            Verdict::T
        } else {
            Verdict::F
        };
        let reason = (verdict == Verdict::F && !definite_violation && !unreachable).then(|| {
            feasible()
                .find_map(|path| path.reason)
                .unwrap_or(UnknownReason::InsufficientInformation)
        });
        Self {
            verdict,
            definite_violation,
            unreachable,
            reason,
            paths,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckReport {
    pub result: bool,
    pub verdict: Verdict,
    pub definite_violation: bool,
    pub unreachable: bool,
    pub reason: Option<UnknownReason>,
    pub paths: Vec<PathProof>,
}

impl CheckReport {
    pub(super) fn new(proof: Proof) -> Self {
        Self {
            result: proof.verdict == Verdict::T,
            verdict: proof.verdict,
            definite_violation: proof.definite_violation,
            unreachable: proof.unreachable,
            reason: proof.reason,
            paths: proof.paths,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_definite_failure_dominates_other_paths() {
        use SolverVerdict::{Proven, Refuted, Unknown, Unreachable};
        for (states, expected, unreachable) in [
            (vec![], Verdict::F, true),
            (vec![Unreachable], Verdict::F, true),
            (vec![Proven, Unreachable], Verdict::T, false),
            (vec![Proven, Proven], Verdict::T, false),
            (vec![Proven, Unknown], Verdict::F, false),
            (vec![Unknown, Unreachable], Verdict::F, false),
            (vec![Refuted, Proven], Verdict::F, false),
            (vec![Refuted, Unknown], Verdict::F, false),
            (vec![Refuted, Unreachable], Verdict::F, false),
        ] {
            for reverse in [false, true] {
                let mut paths: Vec<_> = states
                    .iter()
                    .enumerate()
                    .map(|(id, verdict)| {
                        let result = if *verdict == Unknown {
                            SolveResult::unknown(UnknownReason::BudgetExceeded)
                        } else {
                            SolveResult::known(*verdict)
                        };
                        PathProof::new(id as u32, true, ProofSource::Solver, result)
                    })
                    .collect();
                if reverse {
                    paths.reverse();
                }
                let proof = Proof::from_paths(paths);
                assert_eq!(proof.verdict, expected, "{states:?}");
                assert_eq!(proof.unreachable, unreachable);
                assert_eq!(proof.definite_violation, states.contains(&Refuted));
                assert_eq!(
                    proof.reason.is_some(),
                    expected == Verdict::F && !unreachable && !states.contains(&Refuted)
                );
            }
        }
    }
}
