use console::style;
use fractional_index::FractionalIndex;
use similar::{DiffTag, TextDiff};
use std::collections::BTreeMap;

pub fn generate_custom_diff(
    old_lines: &BTreeMap<FractionalIndex, (String, String)>,
    new_lines: &BTreeMap<FractionalIndex, (String, String)>,
) -> String {
    if old_lines == new_lines {
        return "No changes detected.".to_string();
    }

    let old_keys: Vec<_> = old_lines.keys().cloned().collect();
    let old_values: Vec<_> = old_lines.values().cloned().collect();
    let old_content: Vec<_> = old_values.iter().map(|(c, _s)| c.as_str()).collect();

    let new_keys: Vec<_> = new_lines.keys().cloned().collect();
    let new_values: Vec<_> = new_lines.values().cloned().collect();
    let new_content: Vec<_> = new_values.iter().map(|(c, _s)| c.as_str()).collect();

    const CONTEXT_LINES: usize = 2;
    let diff = TextDiff::from_slices(&old_content, &new_content);

    let mut diff_lines = Vec::new();

    for (hunk_idx, group) in diff.grouped_ops(CONTEXT_LINES).iter().enumerate() {
        if hunk_idx > 0 {
            diff_lines.push("...".to_string());
        }

        let ops: Vec<_> = group.iter().collect();
        let mut i = 0;
        while i < ops.len() {
            let op = ops[i];

            // Default processing if not a whitespace-only modification.
            let (tag, old_range, new_range) = (op.tag(), op.old_range(), op.new_range());
            match tag {
                DiffTag::Replace => {
                    if old_range.len() == 1 && new_range.len() == 1 {
                        let old_line = old_content[old_range.start];
                        let new_line = new_content[new_range.start];
                        let old_normalized: String = old_line.split_whitespace().collect();
                        let new_normalized: String = new_line.split_whitespace().collect();

                        if old_normalized == new_normalized {
                            // Whitespace-only change found. Print the new line as neutral.
                            let (new_text, new_suffix) = &new_values[new_range.start];
                            let lid = crate::file_state::FileState::display_lid(
                                &new_keys[new_range.start],
                                new_suffix,
                            );
                            diff_lines.push(format!("  {lid}: {new_text}"));
                            i += 1;
                            continue;
                        }
                    }

                    for i in old_range {
                        let (old_text, old_suffix) = &old_values[i];
                        let lid =
                            crate::file_state::FileState::display_lid(&old_keys[i], old_suffix);
                        diff_lines.push(style(format!("- {lid}: {old_text}")).red().to_string());
                    }
                    for i in new_range {
                        let (new_text, new_suffix) = &new_values[i];
                        let lid =
                            crate::file_state::FileState::display_lid(&new_keys[i], new_suffix);
                        diff_lines.push(style(format!("+ {lid}: {new_text}")).green().to_string());
                    }
                }
                DiffTag::Delete => {
                    for i in old_range {
                        let (old_text, old_suffix) = &old_values[i];
                        let lid =
                            crate::file_state::FileState::display_lid(&old_keys[i], old_suffix);
                        diff_lines.push(style(format!("- {lid}: {old_text}")).red().to_string());
                    }
                }
                DiffTag::Insert => {
                    for i in new_range {
                        let (new_text, new_suffix) = &new_values[i];
                        let lid =
                            crate::file_state::FileState::display_lid(&new_keys[i], new_suffix);
                        diff_lines.push(style(format!("+ {lid}: {new_text}")).green().to_string());
                    }
                }
                DiffTag::Equal => {
                    for i in new_range {
                        let (new_text, new_suffix) = &new_values[i];
                        let lid =
                            crate::file_state::FileState::display_lid(&new_keys[i], new_suffix);
                        diff_lines.push(format!("  {lid}: {new_text}"));
                    }
                }
            }
            i += 1;
        }
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
        old_lines.insert(idx[0].clone(), ("line 1".to_string(), "".to_string()));
        old_lines.insert(idx[1].clone(), ("line 2".to_string(), "".to_string()));
        old_lines.insert(idx[2].clone(), ("line 3".to_string(), "".to_string()));

        // Case 1: No changes
        let no_change_diff = generate_custom_diff(&old_lines, &old_lines);
        assert_eq!(no_change_diff, "No changes detected.");

        // Case 2: Mix of changes (add, delete, modify)
        let mut new_lines = BTreeMap::new();
        new_lines.insert(idx[0].clone(), ("line 1".to_string(), "".to_string())); // Unchanged, not part of hunk
        new_lines.insert(
            idx[2].clone(),
            ("line 3 modified".to_string(), "".to_string()),
        ); // Modify
        new_lines.insert(idx[3].clone(), ("line 4 added".to_string(), "".to_string())); // Add

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // All changes are contiguous in the master key list, so they form one hunk.
        // Removals first, then additions.
        let expected_lines = [
            format!("  lid-{}_: {}", idx[0].to_string(), "line 1"),
            style(format!("- lid-{}_: {}", idx[1].to_string(), "line 2"))
                .red()
                .to_string(), // Deletion
            style(format!("- lid-{}_: {}", idx[2].to_string(), "line 3"))
                .red()
                .to_string(), // Modification (old)
            style(format!(
                "+ lid-{}_: {}",
                idx[2].to_string(),
                "line 3 modified"
            ))
            .green()
            .to_string(), // Modification (new)
            style(format!("+ lid-{}_: {}", idx[3].to_string(), "line 4 added"))
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
        old_lines.insert(idx[0].clone(), ("  line 1".to_string(), "".to_string()));
        old_lines.insert(idx[1].clone(), ("line 2".to_string(), "".to_string())); // Unchanged

        let mut new_lines = old_lines.clone();
        new_lines.insert(idx[0].clone(), ("line 1".to_string(), "".to_string())); // Whitespace change

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // The neutral line is treated as an "addition" in the hunk.
        let expected_diff = format!(
            "  lid-{}_: {}\n  lid-{}_: {}",
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
        old_lines.insert(idx[0].clone(), ("line A".to_string(), "".to_string()));
        old_lines.insert(idx[1].clone(), ("line B".to_string(), "".to_string())); // To be replaced
        old_lines.insert(idx[2].clone(), ("line C".to_string(), "".to_string())); // To be replaced
        old_lines.insert(idx[3].clone(), ("line D".to_string(), "".to_string()));

        let mut new_lines = BTreeMap::new();
        new_lines.insert(idx[0].clone(), ("line A".to_string(), "".to_string())); // Unchanged
        let new_idx = FractionalIndex::new_between(&idx[1], &idx[2]).unwrap();
        new_lines.insert(new_idx.clone(), ("line X".to_string(), "".to_string())); // Replacement
        new_lines.insert(idx[3].clone(), ("line D".to_string(), "".to_string())); // Unchanged

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  lid-{}_: {}", idx[0].to_string(), "line A"),
            style(format!("- lid-{}_: {}", idx[1].to_string(), "line B"))
                .red()
                .to_string(),
            style(format!("- lid-{}_: {}", idx[2].to_string(), "line C"))
                .red()
                .to_string(),
            style(format!("+ lid-{}_: {}", new_idx.to_string(), "line X"))
                .green()
                .to_string(),
            format!("  lid-{}_: {}", idx[3].to_string(), "line D"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_generate_custom_diff_multiple_hunks() {
        let idx = generate_test_indexes(3);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), ("hunk 1 old".to_string(), "".to_string()));
        old_lines.insert(idx[1].clone(), ("unchanged".to_string(), "".to_string()));
        old_lines.insert(idx[2].clone(), ("hunk 2 old".to_string(), "".to_string()));

        let mut new_lines = BTreeMap::new();
        let new_idx1 = FractionalIndex::new_before(&idx[0]);
        let new_idx2 = FractionalIndex::new_after(&idx[2]);
        new_lines.insert(new_idx1.clone(), ("hunk 1 new".to_string(), "".to_string()));
        new_lines.insert(idx[1].clone(), ("unchanged".to_string(), "".to_string()));
        new_lines.insert(new_idx2.clone(), ("hunk 2 new".to_string(), "".to_string()));

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            // Hunk 1
            style(format!("- lid-{}_: {}", idx[0].to_string(), "hunk 1 old"))
                .red()
                .to_string(),
            style(format!("+ lid-{}_: {}", new_idx1.to_string(), "hunk 1 new"))
                .green()
                .to_string(),
            // Context
            format!("  lid-{}_: {}", idx[1].to_string(), "unchanged"),
            // Hunk 2
            style(format!("- lid-{}_: {}", idx[2].to_string(), "hunk 2 old"))
                .red()
                .to_string(),
            style(format!("+ lid-{}_: {}", new_idx2.to_string(), "hunk 2 new"))
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
        old_lines.insert(idx[0].clone(), ("context 1".to_string(), "".to_string()));
        old_lines.insert(idx[1].clone(), ("context 2".to_string(), "".to_string()));
        old_lines.insert(
            idx[2].clone(),
            ("to be changed".to_string(), "".to_string()),
        );
        old_lines.insert(idx[3].clone(), ("context 3".to_string(), "".to_string()));
        old_lines.insert(idx[4].clone(), ("context 4".to_string(), "".to_string()));

        let mut new_lines = old_lines.clone();
        new_lines.remove(&idx[2]);
        let new_idx = FractionalIndex::new_between(&idx[1], &idx[3]).unwrap();
        new_lines.insert(new_idx.clone(), ("was changed".to_string(), "".to_string()));

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  lid-{}_: {}", idx[0].to_string(), "context 1"),
            format!("  lid-{}_: {}", idx[1].to_string(), "context 2"),
            style(format!(
                "- lid-{}_: {}",
                idx[2].to_string(),
                "to be changed"
            ))
            .red()
            .to_string(),
            style(format!("+ lid-{}_: {}", new_idx.to_string(), "was changed"))
                .green()
                .to_string(),
            format!("  lid-{}_: {}", idx[3].to_string(), "context 3"),
            format!("  lid-{}_: {}", idx[4].to_string(), "context 4"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_hunk_grouping_with_context() {
        let idx = generate_test_indexes(4);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), ("context 1".to_string(), "".to_string()));
        old_lines.insert(idx[1].clone(), ("to change 1".to_string(), "".to_string()));
        old_lines.insert(idx[2].clone(), ("to change 2".to_string(), "".to_string()));
        old_lines.insert(idx[3].clone(), ("context 2".to_string(), "".to_string()));

        let mut new_lines = BTreeMap::new();
        new_lines.insert(idx[0].clone(), ("context 1".to_string(), "".to_string()));
        // Generate new indexes between the old ones to simulate a replacement
        let new_idx1 = FractionalIndex::new(Some(&idx[1]), Some(&idx[2])).unwrap();
        let new_idx2 = FractionalIndex::new(Some(&new_idx1), Some(&idx[2])).unwrap();
        new_lines.insert(new_idx1.clone(), ("changed 1".to_string(), "".to_string()));
        new_lines.insert(new_idx2.clone(), ("changed 2".to_string(), "".to_string()));
        new_lines.insert(idx[3].clone(), ("context 2".to_string(), "".to_string()));

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  lid-{}_: {}", idx[0].to_string(), "context 1"),
            style(format!("- lid-{}_: {}", idx[1].to_string(), "to change 1"))
                .red()
                .to_string(),
            style(format!("- lid-{}_: {}", idx[2].to_string(), "to change 2"))
                .red()
                .to_string(),
            style(format!("+ lid-{}_: {}", new_idx1.to_string(), "changed 1"))
                .green()
                .to_string(),
            style(format!("+ lid-{}_: {}", new_idx2.to_string(), "changed 2"))
                .green()
                .to_string(),
            format!("  lid-{}_: {}", idx[3].to_string(), "context 2"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_whitespace_only_change_with_context() {
        let idx = generate_test_indexes(3);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), ("context".to_string(), "".to_string()));
        old_lines.insert(idx[1].clone(), ("  indented".to_string(), "".to_string()));
        old_lines.insert(idx[2].clone(), ("context".to_string(), "".to_string()));

        let mut new_lines = old_lines.clone();
        new_lines.insert(idx[1].clone(), ("indented".to_string(), "".to_string())); // Whitespace change

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // Should just show the new line as context, without +/-
        let expected_lines = [
            format!("  lid-{}_: {}", idx[0].to_string(), "context"),
            format!("  lid-{}_: {}", idx[1].to_string(), "indented"),
            format!("  lid-{}_: {}", idx[2].to_string(), "context"),
        ];
        let expected_diff = expected_lines.join("\n");
        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_with_overlapping_context() {
        let idx = generate_test_indexes(3);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx[0].clone(), ("hunk 1 old".to_string(), "".to_string()));
        old_lines.insert(idx[1].clone(), ("separator".to_string(), "".to_string()));
        old_lines.insert(idx[2].clone(), ("hunk 2 old".to_string(), "".to_string()));

        let mut new_lines = BTreeMap::new();
        let new_idx1 = FractionalIndex::new_before(&idx[0]);
        let new_idx2 = FractionalIndex::new_after(&idx[2]);
        new_lines.insert(new_idx1.clone(), ("hunk 1 new".to_string(), "".to_string()));
        new_lines.insert(idx[1].clone(), ("separator".to_string(), "".to_string()));
        new_lines.insert(new_idx2.clone(), ("hunk 2 new".to_string(), "".to_string()));

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            style(format!("- lid-{}_: {}", idx[0].to_string(), "hunk 1 old"))
                .red()
                .to_string(),
            style(format!("+ lid-{}_: {}", new_idx1.to_string(), "hunk 1 new"))
                .green()
                .to_string(),
            format!("  lid-{}_: {}", idx[1].to_string(), "separator"),
            style(format!("- lid-{}_: {}", idx[2].to_string(), "hunk 2 old"))
                .red()
                .to_string(),
            style(format!("+ lid-{}_: {}", new_idx2.to_string(), "hunk 2 new"))
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
        old_lines.insert(idx1[0].clone(), ("to change".to_string(), "".to_string()));
        old_lines.insert(idx1[1].clone(), ("context 1".to_string(), "".to_string()));
        old_lines.insert(idx1[2].clone(), ("context 2".to_string(), "".to_string()));

        let mut new_lines = BTreeMap::new();
        let new_idx1 = FractionalIndex::new_before(&idx1[0]);
        new_lines.insert(new_idx1.clone(), ("changed".to_string(), "".to_string()));
        new_lines.insert(idx1[1].clone(), ("context 1".to_string(), "".to_string()));
        new_lines.insert(idx1[2].clone(), ("context 2".to_string(), "".to_string()));

        let diff_start = generate_custom_diff(&old_lines, &new_lines);
        let expected_start = [
            style(format!("- lid-{}_: {}", idx1[0].to_string(), "to change"))
                .red()
                .to_string(),
            style(format!("+ lid-{}_: {}", new_idx1.to_string(), "changed"))
                .green()
                .to_string(),
            format!("  lid-{}_: {}", idx1[1].to_string(), "context 1"),
            format!("  lid-{}_: {}", idx1[2].to_string(), "context 2"),
        ]
        .join("\n");
        assert_eq!(diff_start, expected_start);

        // Case 2: Change at the end
        let idx2 = generate_test_indexes(3);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(idx2[0].clone(), ("context 1".to_string(), "".to_string()));
        old_lines.insert(idx2[1].clone(), ("context 2".to_string(), "".to_string()));
        old_lines.insert(idx2[2].clone(), ("to change".to_string(), "".to_string()));

        let mut new_lines = BTreeMap::new();
        new_lines.insert(idx2[0].clone(), ("context 1".to_string(), "".to_string()));
        new_lines.insert(idx2[1].clone(), ("context 2".to_string(), "".to_string()));
        let new_idx2 = FractionalIndex::new_after(&idx2[2]);
        new_lines.insert(new_idx2.clone(), ("changed".to_string(), "".to_string()));

        let diff_end = generate_custom_diff(&old_lines, &new_lines);
        let expected_end = [
            format!("  lid-{}_: {}", idx2[0].to_string(), "context 1"),
            format!("  lid-{}_: {}", idx2[1].to_string(), "context 2"),
            style(format!("- lid-{}_: {}", idx2[2].to_string(), "to change"))
                .red()
                .to_string(),
            style(format!("+ lid-{}_: {}", new_idx2.to_string(), "changed"))
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
        old_lines.insert(
            idx[0].clone(),
            ("context before".to_string(), "".to_string()),
        );
        old_lines.insert(
            idx[1].clone(),
            ("my_function()".to_string(), "".to_string()),
        );
        old_lines.insert(
            idx[2].clone(),
            ("context after".to_string(), "".to_string()),
        );

        let mut new_lines = old_lines.clone();
        // Change the indentation of the middle line
        new_lines.insert(
            idx[1].clone(),
            ("  my_function()".to_string(), "".to_string()),
        );

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  lid-{}_: {}", idx[0].to_string(), "context before"),
            // The changed line should be neutral, not +/-
            format!("  lid-{}_: {}", idx[1].to_string(), "  my_function()"),
            format!("  lid-{}_: {}", idx[2].to_string(), "context after"),
        ]
        .join("\n");

        // Explicitly check that we don't have the add/remove lines
        assert!(!diff.contains(&format!("- lid-{}_: my_function()", idx[1].to_string())));
        assert!(!diff.contains(&format!("+ lid-{}_:   my_function()", idx[1].to_string())));

        assert_eq!(diff, expected_lines);
    }

    #[test]
    fn test_diff_internal_whitespace_is_neutral() {
        let idx = generate_test_indexes(1);
        let mut old_lines = BTreeMap::new();
        old_lines.insert(
            idx[0].clone(),
            ("fn my_func  (foo: &str) {}".to_string(), "".to_string()),
        );

        let mut new_lines = BTreeMap::new();
        // The only change is the double space to a single space
        new_lines.insert(
            idx[0].clone(),
            ("fn my_func (foo: &str) {}".to_string(), "".to_string()),
        );

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // Should be treated as a whitespace-only change (neutral)
        let expected_lines = [format!(
            "  lid-{}_: {}",
            idx[0].to_string(),
            "fn my_func (foo: &str) {}"
        )];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }
}
