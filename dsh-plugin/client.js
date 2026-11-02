/**
 * MIRSA UI：在右栏 Rust 文件预览中展示可展开的逐行验证结果。
 *
 * 设计约束（踩坑换来的）：
 * - apply 里绝不能让异常冒出去，否则整个 client 条目 failed，页面上什么都不剩。
 * - 使用 documentPreviews 的公开注册接口，复用文件加载、刷新与会话隔离。
 */
window.__ModuleLoader__.load({
  id: 'dsh-mirsa',
  factory(require) {
    const React = require('react');
    const h = React.createElement;

    const KIND = 'mirsa';
    const VERDICTS = {
      violation: { label: '确定违反', color: '#ef4444', bg: 'rgba(239,68,68,0.12)' },
      proved: { label: '已证明', color: '#10b981', bg: 'rgba(16,185,129,0.12)' },
      maybe_violated: { label: '可能违反', color: '#94a3b8', bg: 'rgba(148,163,184,0.14)' },
      unknown: { label: '可能违反', color: '#94a3b8', bg: 'rgba(148,163,184,0.14)' },
      unreachable: { label: '不可达', color: '#a78bfa', bg: 'rgba(167,139,250,0.14)' },
    };
    const mono = {
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
      fontVariantLigatures: 'none',
    };

    function filePathOf(address) {
      const matched = /^dsh-resource:\/\/file\/session\/[^/]+\/(.+)$/.exec(address || '');
      if (!matched) return '';
      try {
        return decodeURIComponent(matched[1]);
      } catch (error) {
        return matched[1];
      }
    }

    const resultStore = (() => {
      const bySession = new Map();
      const listeners = new Set();
      const emit = () => {
        for (const listener of listeners) {
          try { listener(); } catch (error) { /* ignore */ }
        }
      };
      const matches = (source, path) =>
        Boolean(source) && (source === path || source.endsWith('/' + path));
      return {
        set(sessionId, rows) {
          if (!sessionId || !rows || rows.length === 0) return;
          bySession.set(sessionId, rows);
          emit();
        },
        rowsFor(sessionId, path) {
          return (bySession.get(sessionId) || []).filter((row) =>
            matches((row.location && row.location.source) || '', path),
          );
        },
        has(path) {
          for (const rows of bySession.values()) {
            if (rows.some((row) => matches((row.location && row.location.source) || '', path))) return true;
          }
          return false;
        },
        subscribe(listener) {
          listeners.add(listener);
          return () => listeners.delete(listener);
        },
      };
    })();

    const RESULTS = [
      {
        kind: 'violation',
        api: 'slice::get_unchecked',
        dsl: '$index < len($table)',
        line: 7,
        evidence: [['index', '[5, 5]'], ['len(table)', '[3, 3]']],
        code: 'unsafe { *table.get_unchecked(index) }',
      },
      {
        kind: 'proved',
        api: 'slice::get_unchecked',
        dsl: '$index < len($data)',
        line: 13,
        evidence: [['guard', 'index < data.len()']],
        code: 'unsafe { *data.get_unchecked(index) }',
      },
      {
        kind: 'proved',
        api: 'NonZeroU32::new_unchecked',
        dsl: '$x != 0',
        line: 22,
        evidence: [['guard', 'x != 0']],
        code: 'unsafe { NonZeroU32::new_unchecked(x) }',
      },
    ];

    function Pill({ kind }) {
      const tone = VERDICTS[kind] || VERDICTS.unknown;
      return h(
        'span',
        {
          style: {
            display: 'inline-flex',
            alignItems: 'center',
            gap: 5,
            padding: '1px 8px',
            borderRadius: 999,
            background: tone.bg,
            color: tone.color,
            fontSize: 11.5,
            fontWeight: 600,
            whiteSpace: 'nowrap',
          },
        },
        h('span', {
          style: { width: 5, height: 5, borderRadius: 999, background: tone.color },
        }),
        tone.label,
      );
    }

    function Chips({ rows }) {
      if (!rows || rows.length === 0) return null;
      return h(
        'div',
        { style: { display: 'flex', flexWrap: 'wrap', gap: 4, marginTop: 6 } },
        ...rows.map(([name, value], i) =>
          h(
            'span',
            {
              key: i,
              style: {
                ...mono,
                fontSize: 11,
                padding: '3px 7px',
                borderRadius: 5,
                background: 'rgba(127,127,127,0.12)',
                border: '1px solid rgba(127,127,127,0.16)',
                whiteSpace: 'nowrap',
              },
            },
            h('span', { style: { opacity: 0.55 } }, name + ' = '),
            h('span', { style: { fontWeight: 600 } }, value),
          ),
        ),
      );
    }

    function normalize(raw) {
      const verdictMap = {
        violation: 'violation',
        proved: 'proved',
        unreachable: 'unknown',
        unknown: 'unknown',
      };
      const evidence = Array.isArray(raw.evidence)
        ? raw.evidence.map((e) =>
            Array.isArray(e) ? [String(e[0]), String(e[1])] : [String(e.name), String(e.value)],
          )
        : Object.entries(raw.evidence || {});
      return {
        kind: verdictMap[raw.verdict] || 'unknown',
        api: raw.api || '',
        dsl: raw.dsl || '',
        line: raw.line || 0,
        evidence,
        code: raw.code || '',
      };
    }

    function ResultCard({ item }) {
      const tone = VERDICTS[item.kind] || VERDICTS.unknown;
      return h(
        'div',
        {
          style: {
            border: '1px solid rgba(127,127,127,0.16)',
            borderLeft: '3px solid ' + tone.color,
            borderRadius: 9,
            padding: '9px 10px',
            background: 'rgba(127,127,127,0.045)',
          },
        },
        h(
          'div',
          { style: { display: 'flex', alignItems: 'center', gap: 6 } },
          h(Pill, { kind: item.kind }),
          h(
            'span',
            { style: { ...mono, fontSize: 11, fontWeight: 600, opacity: 0.75 } },
            'L' + item.line,
          ),
          h('span', { style: { flex: 1 } }),
          h(
            'span',
            {
              style: {
                ...mono,
                fontSize: 10,
                opacity: 0.5,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                maxWidth: 130,
              },
            },
            item.api,
          ),
        ),
        h(
          'div',
          { style: { ...mono, fontSize: 12.5, fontWeight: 600, marginTop: 7 } },
          item.dsl,
        ),
        h(Chips, { rows: item.evidence }),
        item.code
          ? h(
              'div',
              {
                style: {
                  ...mono,
                  marginTop: 8,
                  padding: '5px 7px',
                  borderRadius: 6,
                  background: 'rgba(127,127,127,0.08)',
                  fontSize: 10.5,
                  opacity: 0.7,
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                },
              },
              item.code,
            )
          : null,
      );
    }

    function SourceView({ source }) {
      const lines = source.split('\n');
      return h(
        'div',
        {
          style: {
            ...mono,
            fontSize: 11,
            lineHeight: '1.55',
            maxHeight: 240,
            overflow: 'auto',
            border: '1px solid rgba(127,127,127,0.18)',
            borderRadius: 8,
            padding: '6px 0',
            background: 'rgba(127,127,127,0.04)',
          },
        },
        ...lines.map((text, i) =>
          h(
            'div',
            { key: i, style: { display: 'flex', gap: 10, padding: '0 10px' } },
            h(
              'span',
              {
                style: {
                  opacity: 0.32,
                  minWidth: 26,
                  textAlign: 'right',
                  userSelect: 'none',
                },
              },
              i + 1,
            ),
            h(
              'span',
              { style: { whiteSpace: 'pre', overflow: 'hidden', textOverflow: 'ellipsis' } },
              text || ' ',
            ),
          ),
        ),
      );
    }

    function findMirsaPayload(value, depth) {
      if (depth > 6 || value === null || value === undefined) return null;
      if (typeof value === 'string') {
        const text = value.trim();
        if (text.startsWith('{') && text.includes('conclusion')) {
          try {
            return JSON.parse(text);
          } catch (error) {
            return null;
          }
        }
        return null;
      }
      if (Array.isArray(value)) {
        for (const item of value) {
          const hit = findMirsaPayload(item, depth + 1);
          if (hit) return hit;
        }
        return null;
      }
      if (typeof value === 'object') {
        for (const item of Object.values(value)) {
          const hit = findMirsaPayload(item, depth + 1);
          if (hit) return hit;
        }
      }
      return null;
    }

    function collectMirsaPayloads(value, depth, out) {
      if (depth > 8 || value === null || value === undefined) return out;
      if (typeof value === 'object' && !Array.isArray(value)) {
        if (Array.isArray(value.reports)) return collectAnalyzePayloads(value, depth, out);
        if (value.conclusion && typeof value.conclusion === 'object') {
          out.push({ ...value, dsl: value.query || value.dsl || '' });
          return out;
        } else if (Array.isArray(value.path_verdicts) && value.verdict !== undefined) {
          out.push({ conclusion: value, location: value.location });
        }
      }
      if (typeof value === 'string') {
        const text = value.trim();
        if (text.startsWith('{')) {
          try {
            const payload = JSON.parse(text);
            collectMirsaPayloads(payload, depth + 1, out);
          } catch (error) {
            // ignore
          }
        }
        return out;
      }
      if (Array.isArray(value)) {
        for (const item of value) collectMirsaPayloads(item, depth + 1, out);
        return out;
      }
      if (typeof value === 'object') {
        for (const item of Object.values(value)) collectMirsaPayloads(item, depth + 1, out);
      }
      return out;
    }

    function findGuard(source, line) {
      if (!source || !line) return '';
      const lines = String(source).split('\n');
      const from = Math.min(line - 2, lines.length - 1);
      for (let i = from; i >= 0 && i > line - 26; i--) {
        const match = /^\s*if\s+(.+?)\s*\{\s*$/.exec(lines[i]);
        if (match) return match[1].trim();
      }
      return '';
    }

    function collectFactsPayloads(value, depth, out) {
      if (depth > 8 || value === null || value === undefined) return out;
      if (typeof value === 'string') {
        try { collectFactsPayloads(JSON.parse(value), depth + 1, out); } catch (error) { /* ignore */ }
        return out;
      }
      if (Array.isArray(value)) {
        for (const item of value) collectFactsPayloads(item, depth + 1, out);
        return out;
      }
      if (typeof value === 'object') {
        if (Array.isArray(value.reports)) {
          for (const report of value.reports) {
            if (report && report.facts) out.push({ ...report, source: value.source });
          }
          return out;
        }
        if (value.facts && value.location) {
          out.push(value);
          return out;
        }
        for (const item of Object.values(value)) collectFactsPayloads(item, depth + 1, out);
      }
      return out;
    }

    function collectAnalyzePayloads(value, depth, out) {
      if (depth > 8 || value === null || value === undefined) return out;
      if (typeof value === 'string') {
        try { collectAnalyzePayloads(JSON.parse(value), depth + 1, out); } catch (error) { /* ignore */ }
        return out;
      }
      if (Array.isArray(value)) {
        for (const item of value) collectAnalyzePayloads(item, depth + 1, out);
        return out;
      }
      if (typeof value !== 'object') return out;
      if (Array.isArray(value.reports) && typeof value.source === 'string') {
        for (const report of value.reports) {
          if (report && report.conclusion) {
            out.push({ ...report, dsl: report.query || '', source: value.source });
            continue;
          }
          if (!report || report.kind !== 'check') continue;
          out.push({
            dsl: report.query || '',
            source: value.source,
            method: report.method || '',
            location: {
              line: report.line || 0,
              source: value.file || '',
              annotated_statement: '',
            },
            conclusion: {
              status: report.status,
              proved: report.status === 'proved',
              definite_violation: report.status === 'violated',
              unreachable: report.status === 'unreachable',
              path_verdicts: [
                {
                  evidence: {
                    method: report.method || '',
                    notes: report.notes || [],
                  },
                  hint: report.hint || undefined,
                },
              ],
            },
          });
        }
        return out;
      }
      for (const item of Object.values(value)) {
        collectAnalyzePayloads(item, depth + 1, out);
      }
      return out;
    }

    // Results belong to this session and this exact source revision, never a basename.
    function previewRows(snapshot, path, text, complete) {
      const unique = new Map();
      for (const node of (snapshot && snapshot.nodes ? snapshot.nodes.values() : [])) {
        const rows = [];
        collectAnalyzePayloads(node, 0, rows);
        for (const row of rows) {
          if (!row.location || row.location.source !== path) continue;
          const key = row.location.line + '|' + row.location.column + '|' + (row.query || row.dsl);
          unique.set(key, row);
        }
      }
      const all = [...unique.values()];
      return {
        rows: complete ? all.filter((row) => typeof row.source === 'string' && row.source === text) : [],
        stale: all.some((row) => typeof row.source !== 'string' || row.source !== text),
      };
    }

    function EvidenceDetails({ payload }) {
      const conclusion = payload.conclusion || {};
      const paths = conclusion.path_verdicts || [];
      return h('div', { style: { whiteSpace: 'normal' } },
        h(MirsaResultCard, { payload }),
        ...paths.map((path, index) => {
          const evidence = path.evidence || {};
          const smt = evidence.smt;
          return h('div', { key: index, style: { padding: '8px 12px', borderLeft: '2px solid rgba(127,127,127,.25)', marginTop: 6 } },
            h('div', { style: { fontWeight: 600 } }, '路径 ' + (path.path_id ?? index) + ' · ' + (evidence.method || path.source || '证据')),
            smt ? h('div', null, '反例查询 ¬P：' + smt.negative + '；性质查询 P：' + smt.positive) : null,
            smt && smt.goal ? h('div', null, '求解目标：' + smt.goal) : null,
            ...(evidence.notes || evidence.context || []).map((note, i) => h('div', { key: i, style: { whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' } }, typeof note === 'string' ? note : JSON.stringify(note))),
            path.hint ? h('div', null, '提示：' + path.hint) : null);
        }),
        h('details', { style: { margin: '8px 12px' } },
          h('summary', { style: { cursor: 'pointer' } }, '原始证据'),
          h('pre', { style: { whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontSize: 11 } }, JSON.stringify(conclusion, null, 2))));
    }

    function MirsaFileView(props) {
      const { content, resourceAddress, useResource, chat } = props;
      const resource = useResource(resourceAddress);
      const path = resource.value && resource.value.absolutePath;
      const snapshot = React.useSyncExternalStore(chat.subscribe, chat.getSnapshot, chat.getSnapshot);
      const [expanded, setExpanded] = React.useState({});
      const text = React.useMemo(() => content.kind === 'bytes' ? new TextDecoder().decode(content.data) : '', [content]);
      React.useEffect(() => setExpanded({}), [resourceAddress, text]);
      if (content.kind !== 'bytes') return null;
      const result = previewRows(snapshot, path, text, true);
      const byLine = new Map();
      for (const row of result.rows) {
        const line = row.location.line;
        if (!byLine.has(line)) byLine.set(line, []);
        byLine.get(line).push(row);
      }
      const nodes = text.split('\n').map((line, index) => {
        const number = index + 1;
        const rows = byLine.get(number) || [];
        const kinds = rows.map((row) => payloadToCard(row).kind);
        const icon = kinds.includes('violation') ? '❌' : kinds.every((kind) => kind === 'proved') ? '✅' : '❔';
        const label = kinds.includes('violation') ? '确定违反' : kinds.every((kind) => kind === 'proved') ? '已证明' : '未定论或不可达';
        const tone = kinds.includes('violation') ? VERDICTS.violation
          : kinds.every((kind) => kind === 'proved') ? VERDICTS.proved
          : VERDICTS.unknown;
        const open = Boolean(expanded[number]);
        return h(React.Fragment, { key: number },
          h('div', { 'data-textpreview-line': number, style: { display: 'flex', alignItems: 'baseline', gap: 8, minHeight: 21, background: rows.length ? tone.bg : 'transparent', borderLeft: '3px solid ' + (rows.length ? tone.color : 'transparent'), borderRadius: rows.length ? 4 : 0, padding: '1px 4px', boxSizing: 'border-box' } },
            h('span', { style: { width: 28, flexShrink: 0, userSelect: 'none' } },
              rows.length ? h('button', {
                type: 'button', title: label + '：点击查看 ' + rows.length + ' 条性质的证据',
                'aria-label': '第 ' + number + ' 行，' + label + '，查看证据',
                'aria-expanded': open,
                onClick: () => setExpanded((old) => ({ ...old, [number]: !old[number] })),
                style: { border: 0, padding: 0, background: 'transparent', cursor: 'pointer', fontSize: 14 },
              }, icon) : null),
            h('span', { style: { width: 38, flexShrink: 0, textAlign: 'right', opacity: 0.4, userSelect: 'none' } }, number),
            h('code', { style: { ...mono, whiteSpace: props.wrap ? 'pre-wrap' : 'pre', overflowWrap: props.wrap ? 'anywhere' : 'normal', flex: 1, minWidth: 0 } }, line || ' ')),
          open ? h('div', { style: { margin: '4px 8px 10px 82px', display: 'grid', gap: 8 } },
            ...rows.map((row, i) => h(EvidenceDetails, { key: i, payload: row }))) : null);
      });
      return h('div', { ref: props.scrollportRef, style: { ...mono, height: '100%', overflow: 'auto', fontSize: 12.5, lineHeight: '21px', padding: '8px', boxSizing: 'border-box' } },
        result.stale ? h('div', { role: 'status', style: { opacity: 0.65, marginBottom: 8 } }, '文件已变化或结果缺少源码快照，旧标记已隐藏；请重新验证。') : null,
        ...nodes);
    }

    function payloadToCard(payload) {
      const conclusion = (payload && payload.conclusion) || {};
      const proved =
        conclusion.proved === true ||
        (conclusion.proved === undefined && conclusion.verdict === 'T');
      const status = conclusion.status;
      const kind =
        status === 'violated'
          ? 'violation'
          : status === 'proved'
            ? 'proved'
            : status === 'unreachable'
              ? 'unreachable'
              : conclusion.definite_violation
                ? 'violation'
                : proved
                  ? 'proved'
                  : conclusion.unreachable
                    ? 'unreachable'
                    : 'maybe_violated';
      const evidence = conclusion.evidence
        ? Object.entries(conclusion.evidence).map(([key, value]) => [
            key,
            Array.isArray(value) ? value.join(' ') : String(value),
          ])
        : [];
      const facts = [...(conclusion.notes || [])];
      let method = '';
      let hint = conclusion.hint || '';
      for (const path of conclusion.path_verdicts || []) {
        for (const note of (path && path.notes) || []) facts.push(String(note));
        const ev = path && path.evidence;
        if (!ev) continue;
        if (!method && ev.method) method = String(ev.method);
        if (!hint && path.hint) hint = String(path.hint);
        for (const item of ev.notes || ev.context || []) facts.push(String(item));
      }
      return {
        kind,
        method,
        hint,
        facts: [...new Set(facts)],
        line: (payload.location && payload.location.line) || 0,
        dsl: payload.query || payload.dsl || '',
        evidence,
        code:
          (payload.location &&
            payload.location.annotated_statement &&
            payload.location.annotated_statement.trim()) ||
          '',
        source_label: (payload.location && payload.location.source) || payload.source_label || '',
      };
    }

    function findMirsaArgs(value, depth) {
      if (depth > 6 || value === null || value === undefined) return null;
      if (typeof value === 'object') {
        if (
          !Array.isArray(value) &&
          typeof value.dsl === 'string' &&
          typeof value.line === 'number'
        ) {
          return value;
        }
        for (const item of Object.values(value)) {
          const hit = findMirsaArgs(item, depth + 1);
          if (hit) return hit;
        }
      }
      return null;
    }

    function CodeWindow({ source, line, color }) {
      const lines = String(source).split('\n');
      const from = Math.max(0, line - 4);
      const to = Math.min(lines.length, line + 3);
      return h(
        'div',
        {
          style: {
            ...mono,
            marginTop: 8,
            borderRadius: 8,
            border: '1px solid rgba(127,127,127,0.16)',
            background: 'rgba(127,127,127,0.05)',
            overflow: 'hidden',
            fontSize: 11,
            lineHeight: '1.6',
          },
        },
        ...lines.slice(from, to).map((text, i) => {
          const number = from + i + 1;
          const active = number === line;
          return h(
            'div',
            {
              key: number,
              style: {
                display: 'flex',
                gap: 10,
                padding: '0 8px',
                background: active ? color + '22' : 'transparent',
                borderLeft: active ? '2px solid ' + color : '2px solid transparent',
              },
            },
            h(
              'span',
              {
                style: {
                  minWidth: 24,
                  textAlign: 'right',
                  opacity: active ? 0.85 : 0.3,
                  userSelect: 'none',
                },
              },
              number,
            ),
            h(
              'span',
              {
                style: {
                  whiteSpace: 'pre',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  opacity: active ? 1 : 0.72,
                  fontWeight: active ? 600 : 400,
                },
              },
              text || ' ',
            ),
          );
        }),
      );
    }

    function MirsaToolCard(props) {
      const payload = findMirsaPayload(props && props.block, 0);
      const args = findMirsaArgs(props && props.block, 0);
      const file = args && typeof args.file === 'string' ? args.file : '';
      const card = payload ? payloadToCard(payload) : null;
      const line =
        (card && card.line) || (args && args.line) || 0;
      const tone = card ? VERDICTS[card.kind] || VERDICTS.unknown : VERDICTS.unknown;
      const [source, setSource] = React.useState('');

      React.useEffect(() => {
        if (!file || !props || typeof props.readText !== 'function') return undefined;
        let cancelled = false;
        props
          .readText(file)
          .then((text) => {
            if (!cancelled) setSource(text);
          })
          .catch(() => {});
        return () => {
          cancelled = true;
        };
      }, [file]);

      const code = card && card.code ? card.code : '';
      return h(
        'div',
        {
          style: {
            margin: '4px 0',
            borderRadius: 12,
            border: '1px solid rgba(127,127,127,0.2)',
            borderLeft: '3px solid ' + tone.color,
            background:
              'linear-gradient(180deg, rgba(127,127,127,0.07), rgba(127,127,127,0.02))',
            overflow: 'hidden',
            boxShadow: card && card.kind === 'violation' ? '0 0 0 1px ' + tone.bg : 'none',
          },
        },
        h(
          'div',
          {
            style: {
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              padding: '8px 12px',
              borderBottom: '1px solid rgba(127,127,127,0.12)',
            },
          },
          h(
            'span',
            {
              style: {
                fontSize: 11,
                fontWeight: 800,
                letterSpacing: '.18em',
                opacity: 0.55,
              },
            },
            'MIRSA',
          ),
          card ? h(Pill, { kind: card.kind }) : h('span', { style: { fontSize: 10.5, opacity: 0.5 } }, '分析中…'),
          h('span', { style: { flex: 1 } }),
          h(
            'span',
            {
              style: {
                ...mono,
                fontSize: 10.5,
                opacity: 0.55,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                maxWidth: 220,
              },
            },
            (file ? file.split('/').pop() : '') + (line ? ':' + line : ''),
          ),
        ),
        h(
          'div',
          { style: { padding: '9px 12px 11px' } },
          (card && card.dsl) || (args && args.dsl) ? h(
            'div',
            {
              style: {
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                marginTop: 2,
              },
            },
            h(
              'span',
              { style: { ...mono, fontSize: 14, fontWeight: 650 } },
              String((card && card.dsl) || args.dsl || '').replace(/\s+at\s+'mirsa_line_entry\s*$/, ''),
            ),
            (card && card.method)
              ? h(
                  'span',
                  {
                    style: {
                      ...mono,
                      fontSize: 10,
                      padding: '1px 6px',
                      borderRadius: 999,
                      border: '1px solid rgba(127,127,127,0.3)',
                      opacity: 0.65,
                      textTransform: 'uppercase',
                      letterSpacing: '.06em',
                    },
                  },
                  card.method,
                )
              : null,
          ) : null,
          card ? h(Chips, { rows: card.evidence }) : null,
          source && line
            ? h(CodeWindow, { source, line, color: tone.color })
            : code
              ? h(
                  'div',
                  {
                    style: {
                      ...mono,
                      marginTop: 8,
                      padding: '5px 8px',
                      borderRadius: 6,
                      background: 'rgba(127,127,127,0.08)',
                      fontSize: 12,
                      opacity: 0.8,
                      whiteSpace: 'pre-wrap',
                    },
                  },
                  code,
                )
              : null,
        ),
      );
    }

    function InlineVerdictCards({ payloads, source, facts }) {
      return h(
        'div',
        { style: { padding: '4px 0 8px 44px' } },
        ...payloads.map((payload, i) => {
          const card = payloadToCard(payload);
          const tone = VERDICTS[card.kind] || VERDICTS.unknown;
          const evidence = card.evidence;
          return h(
            'div',
            {
              key: i,
              style: {
                marginTop: 6,
                borderRadius: 10,
                border: '1px solid rgba(127,127,127,0.18)',
                borderLeft: '3px solid ' + tone.color,
                background:
                  'linear-gradient(180deg, rgba(127,127,127,0.08), rgba(127,127,127,0.02))',
                padding: '8px 10px',
              },
            },
            h(
              'div',
              { style: { display: 'flex', alignItems: 'center', gap: 8 } },
              h(
                'span',
                {
                  style: {
                    fontSize: 9,
                    fontWeight: 800,
                    letterSpacing: '.16em',
                    opacity: 0.5,
                  },
                },
                'MIRSA',
              ),
              h(Pill, { kind: card.kind }),
              h('span', { style: { flex: 1 } }),
              h(
                'span',
                { style: { ...mono, fontSize: 10.5, opacity: 0.55 } },
                'L' + card.line,
              ),
            ),
            h(
              'div',
              { style: { ...mono, fontSize: 12, fontWeight: 650, marginTop: 6 } },
              card.dsl,
            ),
            h(Chips, { rows: evidence }),
            (card.facts || []).length
              ? h(
                  'div',
                  {
                    style: {
                      ...mono,
                      marginTop: 8,
                      fontSize: 11,
                      lineHeight: '1.6',
                      opacity: 0.65,
                      display: 'grid',
                      gap: 3,
                      wordBreak: 'break-all',
                    },
                  },
                  ...(card.facts || []).map((text, index) =>
                    h('div', { key: index }, '· ' + text),
                  ),
                )
              : null,
            (card.hint || '').trim()
              ? h(
                  'div',
                  {
                    style: {
                      marginTop: 7,
                      fontSize: 11,
                      opacity: 0.65,
                      color: tone.color,
                    },
                  },
                  '下一步：' + card.hint,
                )
              : null,
          );
        }),
      );
    }

    function MirsaTurnCards(props) {
      const [tick, setTick] = React.useState(0);
      React.useEffect(() => {
        const timer = setInterval(() => setTick((n) => n + 1), 1000);
        return () => clearInterval(timer);
      }, []);

      let result = { rows: [], total: 0 };
      try {
        result = (props && props.resultsForTurn && props.resultsForTurn(props.turn)) || result;
      } catch (error) {
        result = { rows: [], total: 0, error: String((error && error.message) || error) };
      }
      const rows = result.rows || [];
      const factsByLine = new Map();
      for (const fact of result.facts || []) {
        const line = (fact.location && fact.location.line) || 0;
        if (!line) continue;
        if (!factsByLine.has(line)) factsByLine.set(line, []);
        factsByLine.get(line).push(fact);
      }
      const filePath = (rows[0] && rows[0].location && rows[0].location.source) || '';

      const inlineSource = (rows[0] && rows[0].source) || '';
      const [source, setSource] = React.useState(inlineSource);
      const [sourceError, setSourceError] = React.useState('');
      React.useEffect(() => {
        if (inlineSource) {
          setSource(inlineSource);
          return undefined;
        }
        if (!filePath || !props || typeof props.readText !== 'function') {
          setSourceError('no readText');
          return undefined;
        }
        let cancelled = false;
        props
          .readText(filePath)
          .then((text) => {
            if (!cancelled) {
              setSource(text);
              setSourceError('');
            }
          })
          .catch((error) => {
            if (!cancelled) setSourceError(String((error && error.message) || error).slice(0, 80));
          });
        return () => {
          cancelled = true;
        };
      }, [filePath, inlineSource]);

      React.useEffect(() => {
        if (rows.length) resultStore.set(props && props.sessionId, rows);
      }, [props && props.sessionId, rows.length, filePath]);

      return null;

      if (!source) {
        return h(
          'div',
          { style: { display: 'grid', gap: 8, margin: '10px 0 2px' } },
          sourceError
            ? h(
                'div',
                { style: { ...mono, fontSize: 9.5, opacity: 0.6 } },
                '源码读取失败：' + sourceError,
              )
            : null,
          ...rows.map((payload, i) => h(MirsaResultCard, { key: i, payload })),
        );
      }

      const byLine = new Map();
      for (const payload of rows) {
        const line = (payload.location && payload.location.line) || 0;
        if (!line) continue;
        if (!byLine.has(line)) byLine.set(line, []);
        byLine.get(line).push(payload);
      }

      const lines = source.split('\n');
      const nodes = [];
      lines.forEach((text, index) => {
        const number = index + 1;
        const payloads = byLine.get(number);
        const first = payloads && payloads[0];
        const tone = first
          ? VERDICTS[payloadToCard(first).kind] || VERDICTS.unknown
          : null;
        nodes.push(
          h(
            'div',
            {
              key: 'l' + number,
              style: {
                display: 'flex',
                gap: 12,
                padding: '0 10px',
                background: payloads ? tone.bg : 'transparent',
                borderLeft: payloads ? '2px solid ' + tone.color : '2px solid transparent',
              },
            },
            h(
              'span',
              {
                style: {
                  minWidth: 26,
                  textAlign: 'right',
                  opacity: payloads ? 0.9 : 0.3,
                  userSelect: 'none',
                },
              },
              number,
            ),
            h(
              'span',
              {
                style: {
                  whiteSpace: 'pre',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  fontWeight: payloads ? 600 : 400,
                },
              },
              text || ' ',
            ),
          ),
        );
        if (payloads) {
          nodes.push(
            h(InlineVerdictCards, {
              key: 'c' + number,
              payloads,
              source,
              facts: factsByLine.get(number) || [],
            }),
          );
        }
      });

      return h(
        'div',
        {
          style: {
            margin: '10px 0 2px',
            border: '1px solid rgba(127,127,127,0.2)',
            borderRadius: 12,
            overflow: 'hidden',
            background: 'rgba(127,127,127,0.04)',
          },
        },
        h(
          'div',
          {
            style: {
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              padding: '7px 12px',
              borderBottom: '1px solid rgba(127,127,127,0.14)',
              fontSize: 11.5,
              opacity: 0.6,
            },
          },
          h('span', { style: { fontWeight: 700, letterSpacing: '.1em' } }, 'MIRSA'),
          h('span', { style: { ...mono } }, (filePath || '').split('/').pop()),
          h('span', null, `${rows.length} 处 unsafe`),
        ),
        h(
          'div',
          {
            style: {
              ...mono,
              fontSize: 13,
              lineHeight: '1.75',
              maxHeight: 520,
              overflow: 'auto',
              padding: '6px 0 8px',
            },
          },
          ...nodes,
        ),
      );
    }

    function MirsaResultCard({ payload }) {
      const card = payloadToCard(payload);
      const tone = VERDICTS[card.kind] || VERDICTS.unknown;
      const location = payload.location || {};
      const statement = (location.annotated_statement || card.code || '').trim();
      return h(
        'div',
        {
          style: {
            borderRadius: 12,
            border: '1px solid rgba(127,127,127,0.2)',
            borderLeft: '3px solid ' + tone.color,
            background:
              'linear-gradient(180deg, rgba(127,127,127,0.07), rgba(127,127,127,0.02))',
            overflow: 'hidden',
          },
        },
        h(
          'div',
          {
            style: {
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              padding: '8px 12px',
              borderBottom: '1px solid rgba(127,127,127,0.12)',
            },
          },
          h(
            'span',
            {
              style: {
                fontSize: 11,
                fontWeight: 800,
                letterSpacing: '.18em',
                opacity: 0.55,
              },
            },
            'MIRSA',
          ),
          h(Pill, { kind: card.kind }),
          h('span', { style: { ...mono, fontSize: 12, opacity: 0.7 } }, 'L' + card.line),
          h('span', { style: { flex: 1 } }),
          h(
            'span',
            {
              style: {
                ...mono,
                fontSize: 10,
                opacity: 0.5,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                maxWidth: 220,
              },
            },
            ((location.source || '').split('/').pop() || '') || '',
          ),
        ),
        h(
          'div',
          { style: { padding: '9px 12px 11px' } },
          h(
            'div',
            { style: { ...mono, fontSize: 14, fontWeight: 650 } },
            card.dsl,
          ),
          h(Chips, { rows: card.evidence }),
          statement
            ? h(
                'div',
                {
                  style: {
                    ...mono,
                    marginTop: 8,
                    padding: '6px 8px',
                    borderRadius: 7,
                    background: tone.bg,
                    borderLeft: '2px solid ' + tone.color,
                    fontSize: 11,
                    whiteSpace: 'pre-wrap',
                  },
                },
                statement,
              )
            : null,
        ),
      );
    }

    return {
      inject: ['slots', 'sessions', 'remote', 'remote.workspaceFiles', 'uiWorkspace', 'uiConversation', 'documentPreviews'],
      apply(ctx) {
        ctx.effect(() => ctx.documentPreviews.register({
          id: 'dsh-mirsa/rust', extensions: ['rs'], priority: 'extension',
          title: () => 'Rust · 验证结果', loading: 'bytes-complete', wrap: true,
        }));
        ctx.effect(() => ctx.slots.inject('sidebar.right.tab.document', () =>
          ctx.slots.register({
            name: 'sidebar.right.tab.document', key: 'dsh-mirsa/rust',
            inject: (sessionId) => ({
              chat: ctx.uiConversation.binding(ctx.sessions.binding(sessionId)).target('chat'),
            }),
          }, MirsaFileView)));


      },
    };
  },
});
