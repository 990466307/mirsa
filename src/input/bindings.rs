use super::{
    Error,
    markers::{Markers, Point},
};
use crate::query::{Bindings, Document};
use mirsa_dsl::{Expr, ExprKind};
use rustc_middle::{
    mir::{Body, Operand, VarDebugInfoContents},
    ty::TyCtxt,
};
use std::collections::{BTreeSet, HashMap};

/// Aliases for source variables, resolved separately in each marked scope.
#[derive(Clone, Debug, Default)]
pub struct SourceBindings {
    variables: Vec<(String, String)>,
}

impl SourceBindings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind a DSL name to the visible source variable at each queried marker.
    pub fn variable(mut self, name: impl Into<String>, source_name: impl Into<String>) -> Self {
        self.variables.push((name.into(), source_name.into()));
        self
    }

    pub(super) fn resolve<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        document: &Document,
    ) -> Result<Bindings<'tcx>, Error> {
        let mut aliases = HashMap::new();
        for (name, source) in &self.variables {
            if aliases.insert(name.as_str(), source.as_str()).is_some() {
                return Err(Error::Binding(format!("重复的变量绑定 ${name}")));
            }
        }
        let markers = Markers::new(tcx, body)?;
        let mut bindings = Bindings::new();
        let statement = document.statement();
        let point = markers.resolve(&statement.location, body)?;
        bindings = bindings.location(&statement.location, point.location);
        let mut names = BTreeSet::new();
        referenced_names(&statement.expression, &mut names);
        for name in names {
            let source_name = aliases.get(name).ok_or_else(|| {
                Error::Binding(format!(
                    "变量 ${name} 未绑定，请使用 variable(\"{name}\", \"源码变量名\")"
                ))
            })?;
            let value = visible_value(body, point, source_name)?;
            bindings = bindings.value_at(name, value, &statement.location);
        }
        Ok(bindings)
    }
}

fn visible_value<'tcx>(
    body: &Body<'tcx>,
    point: Point,
    name: &str,
) -> Result<Operand<'tcx>, Error> {
    let mut scopes = Vec::new();
    let mut scope = Some(point.scope);
    while let Some(current) = scope {
        scopes.push(current);
        scope = body.source_scopes[current].parent_scope;
    }
    let mut candidates: Vec<_> = body
        .var_debug_info
        .iter()
        .filter_map(|info| {
            if info.name.as_str() != name
                || info.source_info.span.hi() > point.position
                || info.source_info.span.from_expansion()
            {
                return None;
            }
            let depth = scopes
                .iter()
                .position(|scope| *scope == info.source_info.scope)?;
            Some((depth, info.source_info.span.lo(), info))
        })
        .collect();
    candidates.sort_by_key(|(depth, position, _)| (*depth, std::cmp::Reverse(*position)));
    let Some((depth, position, selected)) = candidates.first() else {
        return Err(Error::Binding(format!("标注处找不到可见的变量 {name}")));
    };
    if candidates
        .iter()
        .skip(1)
        .any(|(d, p, _)| d == depth && p == position)
        || selected.composite.is_some()
    {
        return Err(Error::Binding(format!(
            "变量 {name} 无法唯一映射，请直接使用 MIR 绑定"
        )));
    }
    Ok(match &selected.value {
        VarDebugInfoContents::Place(place) => Operand::Copy(*place),
        VarDebugInfoContents::Const(value) => Operand::Constant(Box::new(value.clone())),
    })
}

fn referenced_names<'a>(expr: &'a Expr, names: &mut BTreeSet<&'a str>) {
    match &expr.kind {
        ExprKind::Binding(name) => {
            names.insert(name);
        }
        ExprKind::Unary(_, expr) | ExprKind::Cast(expr, _) | ExprKind::Field(expr, _) => {
            referenced_names(expr, names)
        }
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
            referenced_names(a, names);
            referenced_names(b, names);
        }
        ExprKind::Builtin(_, args) => {
            for arg in args {
                referenced_names(arg, names);
            }
        }
        _ => {}
    }
}

pub(super) fn unique<T>(mut values: Vec<T>, description: &str) -> Result<T, Error> {
    if values.len() != 1 {
        return Err(Error::Binding(format!(
            "{description} 匹配到 {} 项，需要唯一匹配",
            values.len()
        )));
    }
    Ok(values.pop().unwrap())
}
