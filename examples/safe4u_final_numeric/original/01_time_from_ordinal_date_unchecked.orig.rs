// sample_label: time-rs.time/time/src/date.rs/__from_ordinal_date_unchecked/84
// repo_name: time-rs.time
// relative_file: time/src/date.rs
// function: __from_ordinal_date_unchecked
// lines: 84-94
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub const unsafe fn __from_ordinal_date_unchecked(year: i32, ordinal: u16) -> Self {
        debug_assert!(year >= MIN_YEAR);
        debug_assert!(year <= MAX_YEAR);
        debug_assert!(ordinal != 0);
        debug_assert!(ordinal <= days_in_year(year));

        Self {
            // Safety: The caller must guarantee that `ordinal` is not zero.
            value: unsafe { NonZeroI32::new_unchecked((year << 9) | ordinal as i32) },
        }
    }
