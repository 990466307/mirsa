use mirsa_solver::{SolveResult, Verdict as SolverVerdict};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Proved,
    Violated,
    MaybeViolated,
    Unreachable,
}

impl Status {
    fn from_flags(proved: bool, violated: bool, unreachable: bool) -> Self {
        if violated {
            Self::Violated
        } else if unreachable {
            Self::Unreachable
        } else if proved {
            Self::Proved
        } else {
            Self::MaybeViolated
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Proved => "proved",
            Self::Violated => "violated",
            Self::MaybeViolated => "maybe_violated",
            Self::Unreachable => "unreachable",
        })
    }
}

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
    pub status: Status,
    pub verdict: Verdict,
    pub definite_violation: bool,
    pub unreachable: bool,
    pub source: ProofSource,
    /// Supplemental diagnostics; the property conclusion is explained by evidence.
    pub notes: Vec<String>,
    pub candidate: Option<String>,
    pub evidence: super::Evidence,
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
            status: Status::from_flags(
                result.verdict == SolverVerdict::Proven,
                result.verdict == SolverVerdict::Refuted,
                result.verdict == SolverVerdict::Unreachable,
            ),
            verdict: match result.verdict {
                SolverVerdict::Proven => Verdict::T,
                _ => Verdict::F,
            },
            definite_violation: result.verdict == SolverVerdict::Refuted,
            unreachable: result.verdict == SolverVerdict::Unreachable,
            source,
            notes: Vec::new(),
            candidate: result.candidate,
            evidence: super::Evidence {
                expressions: None,
                abstract_evaluation: None,
                context: Vec::new(),
                smt: None,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Proof {
    pub status: Status,
    pub verdict: Verdict,
    pub definite_violation: bool,
    pub unreachable: bool,
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
        Self {
            status: Status::from_flags(verdict == Verdict::T, definite_violation, unreachable),
            verdict,
            definite_violation,
            unreachable,
            paths,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckReport {
    pub status: Status,
    pub result: bool,
    pub verdict: Verdict,
    pub definite_violation: bool,
    pub unreachable: bool,
    pub paths: Vec<PathProof>,
}

impl CheckReport {
    pub(super) fn new(proof: Proof) -> Self {
        Self {
            status: proof.status,
            result: proof.verdict == Verdict::T,
            verdict: proof.verdict,
            definite_violation: proof.definite_violation,
            unreachable: proof.unreachable,
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
                        let result = SolveResult::known(*verdict);
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
                    proof.status == Status::MaybeViolated,
                    expected == Verdict::F && !unreachable && !states.contains(&Refuted)
                );
            }
        }
    }
}
