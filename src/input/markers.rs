use super::{Error, bindings::unique};
use rustc_hir::{
    self as hir,
    def_id::LocalDefId,
    intravisit::{self, Visitor},
};
use rustc_lexer::TokenKind;
use rustc_middle::{
    mir::{Body, Location, SourceScope},
    ty::TyCtxt,
};
use rustc_span::{BytePos, Span};
use std::collections::HashMap;

struct Marker {
    name: String,
    position: BytePos,
    next: BytePos,
}

#[derive(Clone, Copy)]
pub(super) struct Point {
    pub location: Location,
    pub position: BytePos,
    pub scope: SourceScope,
}

pub(super) struct Markers {
    markers: HashMap<String, Marker>,
    statements: Vec<Span>,
}

impl Markers {
    pub fn find_function(
        tcx: TyCtxt<'_>,
        functions: Vec<LocalDefId>,
        name: &str,
    ) -> Result<LocalDefId, Error> {
        let mut files = HashMap::new();
        let mut candidates = Vec::new();
        for id in functions {
            let body = tcx.hir().body_owned_by(id);
            let span = body.value.span;
            let file = tcx.sess.source_map().lookup_source_file(span.lo());
            let positions = match files.entry(file.start_pos) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let source = file
                        .src
                        .as_ref()
                        .ok_or_else(|| Error::Binding("无法读取标注所在的源码".into()))?;
                    let mut offset = 0;
                    let mut positions = Vec::new();
                    for token in rustc_lexer::tokenize(source) {
                        let start = offset;
                        offset += token.len as usize;
                        if marker_name(&token.kind, &source[start..offset]) == Some(name) {
                            positions.push(file.start_pos + BytePos(start as u32));
                        }
                    }
                    entry.insert(positions)
                }
            };
            let start = positions.partition_point(|&position| position < span.lo());
            let end = positions.partition_point(|&position| position < span.hi());
            if start == end {
                continue;
            }
            let mut collector = Statements {
                tcx,
                spans: Vec::new(),
                nested: Vec::new(),
            };
            collector.visit_body(body);
            if positions[start..end].iter().any(|&position| {
                !collector
                    .nested
                    .iter()
                    .any(|span| span.lo() <= position && position < span.hi())
            }) {
                candidates.push(id);
            }
        }
        match candidates.as_slice() {
            [id] => Ok(*id),
            [] => Err(Error::Binding(format!("找不到源码标注 // mirsa: {name}"))),
            _ => Err(Error::Binding(format!(
                "源码标注 '{name}' 出现在多个函数（{}），请使用唯一的标注名",
                candidates
                    .iter()
                    .map(|id| tcx.def_path_str(id.to_def_id()))
                    .collect::<Vec<_>>()
                    .join("、")
            ))),
        }
    }

    pub fn new(tcx: TyCtxt<'_>, body: &Body<'_>) -> Result<Self, Error> {
        let mut collector = Statements {
            tcx,
            spans: Vec::new(),
            nested: Vec::new(),
        };
        collector.visit_body(tcx.hir().body_owned_by(body.source.def_id().expect_local()));
        let file = tcx.sess.source_map().lookup_source_file(body.span.lo());
        let source = file
            .src
            .as_ref()
            .ok_or_else(|| Error::Binding("无法读取标注所在的源码".into()))?;
        let mut offset = 0;
        let tokens: Vec<_> = rustc_lexer::tokenize(source)
            .map(|token| {
                let start = offset;
                offset += token.len as usize;
                (token.kind, start..offset)
            })
            .collect();
        let mut markers = HashMap::new();
        for (index, (kind, range)) in tokens.iter().enumerate() {
            let position = file.start_pos + BytePos(range.start as u32);
            if position < body.span.lo()
                || position >= body.span.hi()
                || collector
                    .nested
                    .iter()
                    .any(|span| span.lo() <= position && position < span.hi())
            {
                continue;
            }
            let Some(name) = marker_name(kind, &source[range.clone()]) else {
                continue;
            };
            let line_start = source[..range.start].rfind('\n').map_or(0, |i| i + 1);
            if !source[line_start..range.start].trim().is_empty() || !mirsa_dsl::is_name(name) {
                return Err(Error::Binding(
                    "标注必须独占一行，格式为 // mirsa: point（名字使用字母、数字和下划线）".into(),
                ));
            }
            let next = tokens[index + 1..]
                .iter()
                .find(|(kind, _)| {
                    !matches!(
                        kind,
                        TokenKind::Whitespace
                            | TokenKind::LineComment { .. }
                            | TokenKind::BlockComment { .. }
                    )
                })
                .map(|(_, range)| file.start_pos + BytePos(range.start as u32))
                .ok_or_else(|| Error::Binding(format!("标注 '{name}' 后没有语句")))?;
            let marker = Marker {
                name: name.into(),
                position,
                next,
            };
            if markers.insert(name.into(), marker).is_some() {
                return Err(Error::Binding(format!("重复的源码标注 '{name}'")));
            }
        }
        Ok(Self {
            markers,
            statements: collector.spans,
        })
    }

    pub fn resolve(&self, name: &str, body: &Body<'_>) -> Result<Point, Error> {
        let marker = self
            .markers
            .get(name)
            .ok_or_else(|| Error::Binding(format!("找不到源码标注 // mirsa: {name}")))?;
        let target = self
            .statements
            .iter()
            .filter(|s| !s.from_expansion() && s.lo() == marker.next)
            .max_by_key(|s| s.hi())
            .copied()
            .ok_or_else(|| {
                Error::Binding(format!(
                    "标注 '{}' 必须紧接一条语句或块末尾表达式",
                    marker.name
                ))
            })?;
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
            position: marker.position,
            scope: body.source_info(location).scope,
        })
    }
}

fn marker_name<'a>(kind: &TokenKind, text: &'a str) -> Option<&'a str> {
    if matches!(kind, TokenKind::LineComment { doc_style: None }) {
        text[2..].trim().strip_prefix("mirsa:").map(str::trim)
    } else {
        None
    }
}

struct Statements<'tcx> {
    tcx: TyCtxt<'tcx>,
    spans: Vec<Span>,
    nested: Vec<Span>,
}
impl<'tcx> Visitor<'tcx> for Statements<'tcx> {
    fn visit_stmt(&mut self, statement: &'tcx hir::Stmt<'tcx>) {
        self.spans.push(statement.span);
        intravisit::walk_stmt(self, statement);
    }
    fn visit_block(&mut self, block: &'tcx hir::Block<'tcx>) {
        if let Some(expr) = block.expr {
            self.spans.push(expr.span);
        }
        intravisit::walk_block(self, block);
    }
    fn visit_nested_item(&mut self, id: hir::ItemId) {
        self.nested.push(self.tcx.hir().item(id).span);
    }
    fn visit_nested_body(&mut self, id: hir::BodyId) {
        self.nested.push(self.tcx.hir().body(id).value.span);
    }
}
