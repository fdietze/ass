use console::style;
use std::collections::{BTreeMap, BTreeSet};

pub fn generate_custom_diff(
    old_lines: &BTreeMap<u64, String>,
    new_lines: &BTreeMap<u64, String>,
) -> String {
    let old_keys: BTreeSet<_> = old_lines.keys().copied().collect();
    let new_keys: BTreeSet<_> = new_lines.keys().copied().collect();

    let modified_keys: BTreeSet<_> = old_keys
        .intersection(&new_keys)
        .filter(|&k| old_lines.get(k) != new_lines.get(k))
        .copied()
        .collect();

    let changed_keys: BTreeSet<_> = old_keys
        .symmetric_difference(&new_keys)
        .copied()
        .collect::<BTreeSet<_>>()
        .union(&modified_keys)
        .copied()
        .collect();

    if changed_keys.is_empty() {
        return "No changes detected.".to_string();
    }

    const CONTEXT_LINES: usize = 2;
    let mut diff_lines = Vec::new();
    let all_keys: Vec<_> = old_keys.union(&new_keys).copied().collect();

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
            if let Some(idx) = all_keys.iter().position(|&r| r == *key) {
                hunks.push((idx, idx + 1));
            }
        }

        // If still no hunks, it's an unexpected state. Return a simple list of changes.
        if hunks.is_empty() {
            let mut removals = Vec::new();
            let mut additions = Vec::new();
            for key in &changed_keys {
                if let Some(line) = old_lines.get(key) {
                    removals.push(style(format!("- LID{key}: {line}")).red().to_string());
                }
                if let Some(line) = new_lines.get(key) {
                    additions.push(style(format!("+ LID{key}: {line}")).green().to_string());
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
                diff_lines.push(format!("  LID{key}: {line}"));
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
                        hunk_additions.push(format!("  LID{key}: {nv}"));
                    } else {
                        hunk_removals.push(style(format!("- LID{key}: {ov}")).red().to_string());
                        hunk_additions.push(style(format!("+ LID{key}: {nv}")).green().to_string());
                    }
                }
                (Some(ov), None) => {
                    // Deleted
                    hunk_removals.push(style(format!("- LID{key}: {ov}")).red().to_string());
                }
                (None, Some(nv)) => {
                    // Added
                    hunk_additions.push(style(format!("+ LID{key}: {nv}")).green().to_string());
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
                diff_lines.push(format!("  LID{key}: {line}"));
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
    use std::collections::BTreeMap;

    #[test]
    fn test_generate_custom_diff() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "line 1".to_string());
        old_lines.insert(2000, "line 2".to_string());
        old_lines.insert(3000, "line 3".to_string());

        // Case 1: No changes
        let no_change_diff = generate_custom_diff(&old_lines, &old_lines);
        assert_eq!(no_change_diff, "No changes detected.");

        // Case 2: Mix of changes (add, delete, modify)
        let mut new_lines = BTreeMap::new();
        new_lines.insert(1000, "line 1".to_string()); // Unchanged, not part of hunk
        new_lines.insert(3000, "line 3 modified".to_string()); // Modify
        new_lines.insert(4000, "line 4 added".to_string()); // Add

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // All changes are contiguous in the master key list, so they form one hunk.
        // Removals first, then additions.
        let expected_lines = [
            format!("  LID{}: {}", 1000, "line 1"),
            style(format!("- LID{}: {}", 2000, "line 2"))
                .red()
                .to_string(), // Deletion
            style(format!("- LID{}: {}", 3000, "line 3"))
                .red()
                .to_string(), // Modification (old)
            style(format!("+ LID{}: {}", 3000, "line 3 modified"))
                .green()
                .to_string(), // Modification (new)
            style(format!("+ LID{}: {}", 4000, "line 4 added"))
                .green()
                .to_string(), // Addition
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_generate_custom_diff_whitespace_change() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "  line 1".to_string());
        old_lines.insert(2000, "line 2".to_string()); // Unchanged

        let mut new_lines = old_lines.clone();
        new_lines.insert(1000, "line 1".to_string()); // Whitespace change

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // The neutral line is treated as an "addition" in the hunk.
        let expected_diff = format!("  LID{}: {}\n  LID{}: {}", 1000, "line 1", 2000, "line 2");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_generate_custom_diff_hunk_ordering() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "line A".to_string());
        old_lines.insert(2000, "line B".to_string()); // To be replaced
        old_lines.insert(3000, "line C".to_string()); // To be replaced
        old_lines.insert(4000, "line D".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1000, "line A".to_string()); // Unchanged
        new_lines.insert(2500, "line X".to_string()); // Replacement
        new_lines.insert(4000, "line D".to_string()); // Unchanged

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  LID{}: {}", 1000, "line A"),
            style(format!("- LID{}: {}", 2000, "line B"))
                .red()
                .to_string(),
            style(format!("- LID{}: {}", 3000, "line C"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 2500, "line X"))
                .green()
                .to_string(),
            format!("  LID{}: {}", 4000, "line D"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_generate_custom_diff_multiple_hunks() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "hunk 1 old".to_string());
        old_lines.insert(2000, "unchanged".to_string());
        old_lines.insert(3000, "hunk 2 old".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1500, "hunk 1 new".to_string());
        new_lines.insert(2000, "unchanged".to_string());
        new_lines.insert(3500, "hunk 2 new".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            // Hunk 1
            style(format!("- LID{}: {}", 1000, "hunk 1 old"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 1500, "hunk 1 new"))
                .green()
                .to_string(),
            // Context
            format!("  LID{}: {}", 2000, "unchanged"),
            // Hunk 2
            style(format!("- LID{}: {}", 3000, "hunk 2 old"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 3500, "hunk 2 new"))
                .green()
                .to_string(),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    // --- Start of added tests for context diff ---

    #[test]
    fn test_diff_with_basic_context() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "context 1".to_string());
        old_lines.insert(2000, "context 2".to_string());
        old_lines.insert(3000, "to be changed".to_string());
        old_lines.insert(4000, "context 3".to_string());
        old_lines.insert(5000, "context 4".to_string());

        let mut new_lines = old_lines.clone();
        new_lines.remove(&3000);
        new_lines.insert(3500, "was changed".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  LID{}: {}", 1000, "context 1"),
            format!("  LID{}: {}", 2000, "context 2"),
            style(format!("- LID{}: {}", 3000, "to be changed"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 3500, "was changed"))
                .green()
                .to_string(),
            format!("  LID{}: {}", 4000, "context 3"),
            format!("  LID{}: {}", 5000, "context 4"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_hunk_grouping_with_context() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "context 1".to_string());
        old_lines.insert(2000, "to change 1".to_string());
        old_lines.insert(3000, "to change 2".to_string());
        old_lines.insert(4000, "context 2".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1000, "context 1".to_string());
        new_lines.insert(2500, "changed 1".to_string());
        new_lines.insert(2600, "changed 2".to_string());
        new_lines.insert(4000, "context 2".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  LID{}: {}", 1000, "context 1"),
            style(format!("- LID{}: {}", 2000, "to change 1"))
                .red()
                .to_string(),
            style(format!("- LID{}: {}", 3000, "to change 2"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 2500, "changed 1"))
                .green()
                .to_string(),
            style(format!("+ LID{}: {}", 2600, "changed 2"))
                .green()
                .to_string(),
            format!("  LID{}: {}", 4000, "context 2"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_whitespace_only_change_with_context() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "context".to_string());
        old_lines.insert(2000, "  indented".to_string());
        old_lines.insert(3000, "context".to_string());

        let mut new_lines = old_lines.clone();
        new_lines.insert(2000, "indented".to_string()); // Whitespace change

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // Should just show the new line as context, without +/-
        let expected_lines = [
            format!("  LID{}: {}", 1000, "context"),
            format!("  LID{}: {}", 2000, "indented"),
            format!("  LID{}: {}", 3000, "context"),
        ];
        let expected_diff = expected_lines.join("\n");
        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_with_overlapping_context() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "hunk 1 old".to_string());
        old_lines.insert(2000, "separator".to_string());
        old_lines.insert(3000, "hunk 2 old".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1500, "hunk 1 new".to_string());
        new_lines.insert(2000, "separator".to_string());
        new_lines.insert(3500, "hunk 2 new".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            style(format!("- LID{}: {}", 1000, "hunk 1 old"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 1500, "hunk 1 new"))
                .green()
                .to_string(),
            format!("  LID{}: {}", 2000, "separator"),
            style(format!("- LID{}: {}", 3000, "hunk 2 old"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 3500, "hunk 2 new"))
                .green()
                .to_string(),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_at_file_boundaries() {
        // Case 1: Change at the beginning
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "to change".to_string());
        old_lines.insert(2000, "context 1".to_string());
        old_lines.insert(3000, "context 2".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1500, "changed".to_string());
        new_lines.insert(2000, "context 1".to_string());
        new_lines.insert(3000, "context 2".to_string());

        let diff_start = generate_custom_diff(&old_lines, &new_lines);
        let expected_start = [
            style(format!("- LID{}: {}", 1000, "to change"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 1500, "changed"))
                .green()
                .to_string(),
            format!("  LID{}: {}", 2000, "context 1"),
            format!("  LID{}: {}", 3000, "context 2"),
        ]
        .join("\n");
        assert_eq!(diff_start, expected_start);

        // Case 2: Change at the end
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "context 1".to_string());
        old_lines.insert(2000, "context 2".to_string());
        old_lines.insert(3000, "to change".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1000, "context 1".to_string());
        new_lines.insert(2000, "context 2".to_string());
        new_lines.insert(3500, "changed".to_string());

        let diff_end = generate_custom_diff(&old_lines, &new_lines);
        let expected_end = [
            format!("  LID{}: {}", 1000, "context 1"),
            format!("  LID{}: {}", 2000, "context 2"),
            style(format!("- LID{}: {}", 3000, "to change"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 3500, "changed"))
                .green()
                .to_string(),
        ]
        .join("\n");
        assert_eq!(diff_end, expected_end);
    }

    #[test]
    fn test_diff_whitespace_change_is_neutral() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "context before".to_string());
        old_lines.insert(2000, "my_function()".to_string());
        old_lines.insert(3000, "context after".to_string());

        let mut new_lines = old_lines.clone();
        // Change the indentation of the middle line
        new_lines.insert(2000, "  my_function()".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  LID{}: {}", 1000, "context before"),
            // The changed line should be neutral, not +/-
            format!("  LID{}: {}", 2000, "  my_function()"),
            format!("  LID{}: {}", 3000, "context after"),
        ]
        .join("\n");

        // Explicitly check that we don't have the add/remove lines
        assert!(!diff.contains("- LID2000: my_function()"));
        assert!(!diff.contains("+ LID2000:   my_function()"));

        assert_eq!(diff, expected_lines);
    }

    #[test]
    fn test_diff_internal_whitespace_is_neutral() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "fn my_func  (foo: &str) {}".to_string());

        let mut new_lines = BTreeMap::new();
        // The only change is the double space to a single space
        new_lines.insert(1000, "fn my_func (foo: &str) {}".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // Should be treated as a whitespace-only change (neutral)
        let expected_lines = [format!("  LID{}: {}", 1000, "fn my_func (foo: &str) {}")];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }
}
