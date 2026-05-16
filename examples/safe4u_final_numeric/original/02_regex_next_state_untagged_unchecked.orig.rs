// sample_label: rust-lang.regex/regex-automata/src/hybrid/dfa.rs/next_state_untagged_unchecked/1414
// repo_name: rust-lang.regex
// relative_file: regex-automata/src/hybrid/dfa.rs
// function: next_state_untagged_unchecked
// lines: 1414-1424
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn next_state_untagged_unchecked(
        &self,
        cache: &Cache,
        current: LazyStateID,
        input: u8,
    ) -> LazyStateID {
        debug_assert!(!current.is_tagged());
        let class = usize::from(self.classes.get(input));
        let offset = current.as_usize_unchecked() + class;
        *cache.trans.get_unchecked(offset)
    }
