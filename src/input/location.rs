use super::{Error, bindings::unique};
use rustc_middle::mir::{Body, Location, SourceScope};
use rustc_span::{BytePos, Span};

#[derive(Clone, Copy)]
pub(super) struct Point {
    pub location: Location,
    pub position: BytePos,
    pub scope: SourceScope,
}

pub(super) fn at_span(
    body: &Body<'_>,
    target: Span,
    position: BytePos,
    name: &str,
) -> Result<Point, Error> {
    let dominators = body.basic_blocks.dominators();
    // One source statement can lower to many MIR operations. Use its unique
    // entry, not whichever operation happens to appear first in block order.
    let mut entries = Vec::new();
    for (block, data) in body.basic_blocks.iter_enumerated() {
        if data.is_cleanup || !dominators.is_reachable(block) {
            continue;
        }
        for index in 0..=data.statements.len() {
            let location = Location {
                block,
                statement_index: index,
            };
            let span = body.source_info(location).span;
            if !span.is_dummy() && !span.from_expansion() && target.contains(span) {
                entries.push(location);
                break;
            }
        }
    }
    let candidates: Vec<_> = entries
        .iter()
        .copied()
        .filter(|entry| {
            entries
                .iter()
                .all(|other| dominators.dominates(entry.block, other.block))
        })
        .collect();
    let location = unique(
        candidates,
        &format!("标注 '{name} 的 MIR 语句入口（语句可能已消除或无法唯一映射）"),
    )?;
    Ok(Point {
        location,
        position,
        scope: body.source_info(location).scope,
    })
}
