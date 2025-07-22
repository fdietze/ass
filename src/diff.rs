use console::style;
use fractional_index::FractionalIndex;
use std::collections::{BTreeMap, BTreeSet};

pub fn generate_custom_diff(
    old_lines: &BTreeMap<FractionalIndex, String>,
    new_lines: &BTreeMap<FractionalIndex, String>,
) -> String {
    let old_keys: BTreeSet<_> = old_lines.keys().cloned().collect();
    let new_keys: BTreeSet<_> = new_lines.keys().cloned().collect();

    let modified_keys: BTreeSet<_> = old_keys
        .intersection(&new_keys)
        .filter(|&k| old_lines.get(k) != new_lines.get(k))
        .cloned()
        .collect();

    let changed_keys: BTreeSet<_> = old_keys
        .symmetric_difference(&new_keys)
        .cloned()
        .collect::<BTreeSet<_>>()
        .union(&modified_keys)
        .cloned()
        .collect();

    if changed_keys.is_empty() {
        return "No changes detected.".to_string();
    }

    const CONTEXT_LINES: usize = 2;
    let mut diff_lines = Vec::new();
    let all_keys: Vec<_> = old_keys.union(&new_keys).cloned().collect();

    // 1. Identify hunks (contiguous blocks of changed keys)
    let mut hunks: Vec<(usize, usize)> = vec![];
    if !all_keys.is_empty() {
        let mut i = 0;
        while i < all_keys.len() {
            if changed_keys.contains(&all_keys[i]) {
                let start = i;
                while i < all_keys.len() && changed_keys.contains(&all_keys[i]) {
                    i += 1;
                }
                hunks.push((start, i));
            } else {
                i += 1;
            }
        }
    }

    // If there are no hunks, but there are changes, it's a logic error.
    // But if there are no changes, we should have already returned.
    // If there are hunks, proceed.
    if hunks.is_empty() && !changed_keys.is_empty() {
        // This case can happen if, for example, a whitespace-only change is the ONLY change.
        // The old logic for whitespace changes was to treat them as additions, which the new logic
        // will now handle inside the hunk processing. Let's find the modified key.
        if let Some(key) = modified_keys.iter().next() {
            if let Some(idx) = all_keys.iter().position(|r| r == key) {
                hunks.push((idx, idx + 1));
            }
        }

        // If still no hunks, it's an unexpected state. Return a simple list of changes.
        if hunks.is_empty() {
            let mut removals = Vec::new();
            let mut additions = Vec::new();
            for key in &changed_keys {
                if let Some(line) = old_lines.get(key) {
                    removals.push(
                        style(format!("- {}: {line}", key.to_string()))
                            .red()
                            .to_string(),
                    );
                }
                if let Some(line) = new_lines.get(key) {
                    additions.push(
                        style(format!("+ {}: {line}", key.to_string()))
                            .green()
                            .to_string(),
                    );
                }
            }
            removals.extend(additions);
            return removals.join("\n");
        }
    }

    // 2. Render hunks with context
    let mut last_printed_index: Option<usize> = None;

    for (hunk_start, hunk_end) in hunks {
        let context_start = hunk_start.saturating_sub(CONTEXT_LINES);
        let print_start = if let Some(last_idx) = last_printed_index {
            // If the new hunk's context overlaps with or is adjacent to the last one,
            // we start right after the last printed line.
            // Otherwise, we print a separator.
            if context_start > last_idx {
                diff_lines.push("...".to_string());
                context_start
            } else {
                last_idx
            }
        } else {
            context_start
        };

        // Pre-hunk context
        for key in all_keys.iter().take(hunk_start).skip(print_start) {
            if let Some(line) = new_lines.get(key) {
                diff_lines.push(format!("  {}: {line}", key.to_string()));
            }
        }

        // The hunk itself
        let mut hunk_removals = Vec::new();
        let mut hunk_additions = Vec::new();
        for key in all_keys.iter().take(hunk_end).skip(hunk_start) {
            let old_val = old_lines.get(key);
            let new_val = new_lines.get(key);
            match (old_val, new_val) {
                (Some(ov), Some(nv)) => {
                    // Modified
                    let old_normalized: String = ov.split_whitespace().collect();
                    let new_normalized: String = nv.split_whitespace().collect();

                    if old_normalized == new_normalized {
                        hunk_additions.push(format!("  {}: {nv}", key.to_string()));
                    } else {
                        hunk_removals.push(
                            style(format!("- {}: {ov}", key.to_string()))
                                .red()
                                .to_string(),
                        );
                        hunk_additions.push(
                            style(format!("+ {}: {nv}", key.to_string()))
                                .green()
                                .to_string(),
                        );
                    }
                }
                (Some(ov), None) => {
                    // Deleted
                    hunk_removals.push(
                        style(format!("- {}: {ov}", key.to_string()))
                            .red()
                            .to_string(),
                    );
                }
                (None, Some(nv)) => {
                    // Added
                    hunk_additions.push(
                        style(format!("+ {}: {nv}", key.to_string()))
                            .green()
                            .to_string(),
                    );
                }
                (None, None) => unreachable!(),
            }
        }
        diff_lines.extend(hunk_removals);
        diff_lines.extend(hunk_additions);

        // Post-hunk context
        let context_end = (hunk_end + CONTEXT_LINES).min(all_keys.len());
        for key in all_keys.iter().take(context_end).skip(hunk_end) {
            if let Some(line) = new_lines.get(key) {
                diff_lines.push(format!("  {}: {line}", key.to_string()));
            }
        }
        last_printed_index = Some(context_end);
    }

    diff_lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use console::style;
    use fractional_index::FractionalIndex;
    use std::collections::BTreeMap;

    // Helper to generate a sequence of valid fractional indexes for testing.
    fn generate_test_indexes(count: usize) -> Vec<FractionalIndex> {
        let mut indexes = Vec::new();
        if count == 0 {
            return indexes;
        }
        let mut last_index = FractionalIndex::default();
        indexes.push(last_index.clone());
        for _ in 1..count {
            last_index = FractionalIndex::new_after(&last_index);
            indexes.push(last_index.clone());
        }
        indexes
    }

    #[test]
    fn test_generate_custom_diff() {
        let idx = generate_test_indexes(4);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), "line 1".to_string());
        old_lines.insert(idx[1].clone(), "line 2".to_string());
        old_lines.insert(idx[2].clone(), "line 3".to_string());

        // Case 1: No changes
        let no_change_diff = generate_custom_diff(&old_lines, &old_lines);
        assert_eq!(no_change_diff, "No changes detected.");

        // Case 2: Mix of changes (add, delete, modify)
        let mut new_lines = BTreeMap::new();
        new_lines.insert(idx[0].clone(), "line 1".to_string()); // Unchanged, not part of hunk
        new_lines.insert(idx[2].clone(), "line 3 modified".to_string()); // Modify
        new_lines.insert(idx[3].clone(), "line 4 added".to_string()); // Add

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // All changes are contiguous in the master key list, so they form one hunk.
        // Removals first, then additions.
        let expected_lines = [
            format!("  {}: {}", idx[0].to_string(), "line 1"),
            style(format!("- {}: {}", idx[1].to_string(), "line 2"))
                .red()
                .to_string(), // Deletion
            style(format!("- {}: {}", idx[2].to_string(), "line 3"))
                .red()
                .to_string(), // Modification (old)
            style(format!("+ {}: {}", idx[2].to_string(), "line 3 modified"))
                .green()
                .to_string(), // Modification (new)
            style(format!("+ {}: {}", idx[3].to_string(), "line 4 added"))
                .green()
                .to_string(), // Addition
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_generate_custom_diff_whitespace_change() {
        let idx = generate_test_indexes(2);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), "  line 1".to_string());
        old_lines.insert(idx[1].clone(), "line 2".to_string()); // Unchanged

        let mut new_lines = old_lines.clone();
        new_lines.insert(idx[0].clone(), "line 1".to_string()); // Whitespace change

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // The neutral line is treated as an "addition" in the hunk.
        let expected_diff = format!(
            "  {}: {}\n  {}: {}",
            idx[0].to_string(),
            "line 1",
            idx[1].to_string(),
            "line 2"
        );

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_generate_custom_diff_hunk_ordering() {
        let idx = generate_test_indexes(4);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), "line A".to_string());
        old_lines.insert(idx[1].clone(), "line B".to_string()); // To be replaced
        old_lines.insert(idx[2].clone(), "line C".to_string()); // To be replaced
        old_lines.insert(idx[3].clone(), "line D".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(idx[0].clone(), "line A".to_string()); // Unchanged
        let new_idx = FractionalIndex::new_between(&idx[1], &idx[2]).unwrap();
        new_lines.insert(new_idx.clone(), "line X".to_string()); // Replacement
        new_lines.insert(idx[3].clone(), "line D".to_string()); // Unchanged

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  {}: {}", idx[0].to_string(), "line A"),
            style(format!("- {}: {}", idx[1].to_string(), "line B"))
                .red()
                .to_string(),
            style(format!("- {}: {}", idx[2].to_string(), "line C"))
                .red()
                .to_string(),
            style(format!("+ {}: {}", new_idx.to_string(), "line X"))
                .green()
                .to_string(),
            format!("  {}: {}", idx[3].to_string(), "line D"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_generate_custom_diff_multiple_hunks() {
        let idx = generate_test_indexes(3);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), "hunk 1 old".to_string());
        old_lines.insert(idx[1].clone(), "unchanged".to_string());
        old_lines.insert(idx[2].clone(), "hunk 2 old".to_string());

        let mut new_lines = BTreeMap::new();
        let new_idx1 = FractionalIndex::new_before(&idx[0]);
        let new_idx2 = FractionalIndex::new_after(&idx[2]);
        new_lines.insert(new_idx1.clone(), "hunk 1 new".to_string());
        new_lines.insert(idx[1].clone(), "unchanged".to_string());
        new_lines.insert(new_idx2.clone(), "hunk 2 new".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            // Hunk 1
            style(format!("- {}: {}", idx[0].to_string(), "hunk 1 old"))
                .red()
                .to_string(),
            style(format!("+ {}: {}", new_idx1.to_string(), "hunk 1 new"))
                .green()
                .to_string(),
            // Context
            format!("  {}: {}", idx[1].to_string(), "unchanged"),
            // Hunk 2
            style(format!("- {}: {}", idx[2].to_string(), "hunk 2 old"))
                .red()
                .to_string(),
            style(format!("+ {}: {}", new_idx2.to_string(), "hunk 2 new"))
                .green()
                .to_string(),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    // --- Start of added tests for context diff ---

    #[test]
    fn test_diff_with_basic_context() {
        let idx = generate_test_indexes(5);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), "context 1".to_string());
        old_lines.insert(idx[1].clone(), "context 2".to_string());
        old_lines.insert(idx[2].clone(), "to be changed".to_string());
        old_lines.insert(idx[3].clone(), "context 3".to_string());
        old_lines.insert(idx[4].clone(), "context 4".to_string());

        let mut new_lines = old_lines.clone();
        new_lines.remove(&idx[2]);
        let new_idx = FractionalIndex::new_between(&idx[1], &idx[3]).unwrap();
        new_lines.insert(new_idx.clone(), "was changed".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  {}: {}", idx[0].to_string(), "context 1"),
            format!("  {}: {}", idx[1].to_string(), "context 2"),
            style(format!("- {}: {}", idx[2].to_string(), "to be changed"))
                .red()
                .to_string(),
            style(format!("+ {}: {}", new_idx.to_string(), "was changed"))
                .green()
                .to_string(),
            format!("  {}: {}", idx[3].to_string(), "context 3"),
            format!("  {}: {}", idx[4].to_string(), "context 4"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_hunk_grouping_with_context() {
        let idx = generate_test_indexes(4);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), "context 1".to_string());
        old_lines.insert(idx[1].clone(), "to change 1".to_string());
        old_lines.insert(idx[2].clone(), "to change 2".to_string());
        old_lines.insert(idx[3].clone(), "context 2".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(idx[0].clone(), "context 1".to_string());
        // Generate new indexes between the old ones to simulate a replacement
        let new_idx1 = FractionalIndex::new(Some(&idx[1]), Some(&idx[2])).unwrap();
        let new_idx2 = FractionalIndex::new(Some(&new_idx1), Some(&idx[2])).unwrap();
        new_lines.insert(new_idx1.clone(), "changed 1".to_string());
        new_lines.insert(new_idx2.clone(), "changed 2".to_string());
        new_lines.insert(idx[3].clone(), "context 2".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  {}: {}", idx[0].to_string(), "context 1"),
            style(format!("- {}: {}", idx[1].to_string(), "to change 1"))
                .red()
                .to_string(),
            style(format!("- {}: {}", idx[2].to_string(), "to change 2"))
                .red()
                .to_string(),
            style(format!("+ {}: {}", new_idx1.to_string(), "changed 1"))
                .green()
                .to_string(),
            style(format!("+ {}: {}", new_idx2.to_string(), "changed 2"))
                .green()
                .to_string(),
            format!("  {}: {}", idx[3].to_string(), "context 2"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_whitespace_only_change_with_context() {
        let idx = generate_test_indexes(3);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), "context".to_string());
        old_lines.insert(idx[1].clone(), "  indented".to_string());
        old_lines.insert(idx[2].clone(), "context".to_string());

        let mut new_lines = old_lines.clone();
        new_lines.insert(idx[1].clone(), "indented".to_string()); // Whitespace change

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // Should just show the new line as context, without +/-
        let expected_lines = [
            format!("  {}: {}", idx[0].to_string(), "context"),
            format!("  {}: {}", idx[1].to_string(), "indented"),
            format!("  {}: {}", idx[2].to_string(), "context"),
        ];
        let expected_diff = expected_lines.join("\n");
        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_with_overlapping_context() {
        let idx = generate_test_indexes(3);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), "hunk 1 old".to_string());
        old_lines.insert(idx[1].clone(), "separator".to_string());
        old_lines.insert(idx[2].clone(), "hunk 2 old".to_string());

        let mut new_lines = BTreeMap::new();
        let new_idx1 = FractionalIndex::new_before(&idx[0]);
        let new_idx2 = FractionalIndex::new_after(&idx[2]);
        new_lines.insert(new_idx1.clone(), "hunk 1 new".to_string());
        new_lines.insert(idx[1].clone(), "separator".to_string());
        new_lines.insert(new_idx2.clone(), "hunk 2 new".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            style(format!("- {}: {}", idx[0].to_string(), "hunk 1 old"))
                .red()
                .to_string(),
            style(format!("+ {}: {}", new_idx1.to_string(), "hunk 1 new"))
                .green()
                .to_string(),
            format!("  {}: {}", idx[1].to_string(), "separator"),
            style(format!("- {}: {}", idx[2].to_string(), "hunk 2 old"))
                .red()
                .to_string(),
            style(format!("+ {}: {}", new_idx2.to_string(), "hunk 2 new"))
                .green()
                .to_string(),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_at_file_boundaries() {
        // Case 1: Change at the beginning
        let idx1 = generate_test_indexes(3);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx1[0].clone(), "to change".to_string());
        old_lines.insert(idx1[1].clone(), "context 1".to_string());
        old_lines.insert(idx1[2].clone(), "context 2".to_string());

        let mut new_lines = BTreeMap::new();
        let new_idx1 = FractionalIndex::new_before(&idx1[0]);
        new_lines.insert(new_idx1.clone(), "changed".to_string());
        new_lines.insert(idx1[1].clone(), "context 1".to_string());
        new_lines.insert(idx1[2].clone(), "context 2".to_string());

        let diff_start = generate_custom_diff(&old_lines, &new_lines);
        let expected_start = [
            style(format!("- {}: {}", idx1[0].to_string(), "to change"))
                .red()
                .to_string(),
            style(format!("+ {}: {}", new_idx1.to_string(), "changed"))
                .green()
                .to_string(),
            format!("  {}: {}", idx1[1].to_string(), "context 1"),
            format!("  {}: {}", idx1[2].to_string(), "context 2"),
        ]
        .join("\n");
        assert_eq!(diff_start, expected_start);

        // Case 2: Change at the end
        let idx2 = generate_test_indexes(3);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx2[0].clone(), "context 1".to_string());
        old_lines.insert(idx2[1].clone(), "context 2".to_string());
        old_lines.insert(idx2[2].clone(), "to change".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(idx2[0].clone(), "context 1".to_string());
        new_lines.insert(idx2[1].clone(), "context 2".to_string());
        let new_idx2 = FractionalIndex::new_after(&idx2[2]);
        new_lines.insert(new_idx2.clone(), "changed".to_string());

        let diff_end = generate_custom_diff(&old_lines, &new_lines);
        let expected_end = [
            format!("  {}: {}", idx2[0].to_string(), "context 1"),
            format!("  {}: {}", idx2[1].to_string(), "context 2"),
            style(format!("- {}: {}", idx2[2].to_string(), "to change"))
                .red()
                .to_string(),
            style(format!("+ {}: {}", new_idx2.to_string(), "changed"))
                .green()
                .to_string(),
        ]
        .join("\n");
        assert_eq!(diff_end, expected_end);
    }

    #[test]
    fn test_diff_whitespace_change_is_neutral() {
        let idx = generate_test_indexes(3);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), "context before".to_string());
        old_lines.insert(idx[1].clone(), "my_function()".to_string());
        old_lines.insert(idx[2].clone(), "context after".to_string());

        let mut new_lines = old_lines.clone();
        // Change the indentation of the middle line
        new_lines.insert(idx[1].clone(), "  my_function()".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  {}: {}", idx[0].to_string(), "context before"),
            // The changed line should be neutral, not +/-
            format!("  {}: {}", idx[1].to_string(), "  my_function()"),
            format!("  {}: {}", idx[2].to_string(), "context after"),
        ]
        .join("\n");

        // Explicitly check that we don't have the add/remove lines
        assert!(!diff.contains(&format!("- {}: my_function()", idx[1].to_string())));
        assert!(!diff.contains(&format!("+ {}:   my_function()", idx[1].to_string())));

        assert_eq!(diff, expected_lines);
    }

    #[test]
    fn test_diff_internal_whitespace_is_neutral() {
        let idx = generate_test_indexes(1);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), "fn my_func  (foo: &str) {}".to_string());

        let mut new_lines = BTreeMap::new();
        // The only change is the double space to a single space
        new_lines.insert(idx[0].clone(), "fn my_func (foo: &str) {}".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // Should be treated as a whitespace-only change (neutral)
        let expected_lines = [format!(
            "  {}: {}",
            idx[0].to_string(),
            "fn my_func (foo: &str) {}"
        )];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }
}
