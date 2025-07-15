use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub struct ExpansionResult {
    pub files: Vec<String>,
    pub not_found: Vec<String>,
}

pub fn expand_and_validate(paths: &[String], ignored_paths: &[String]) -> ExpansionResult {
    let mut files = BTreeSet::new();
    let mut not_found = Vec::new();

    let mut override_builder = OverrideBuilder::new(std::env::current_dir().unwrap());
    for pattern in ignored_paths {
        // We add a `!` prefix to make it an ignore pattern.
        let ignore_pattern = format!("!{pattern}");
        override_builder.add(&ignore_pattern).unwrap();
    }
    let overrides = override_builder.build().unwrap();

    for path_str in paths {
        let path = Path::new(path_str);
        if path.exists() {
            for entry in WalkBuilder::new(path)
                .hidden(false)
                .overrides(overrides.clone())
                .build()
                .flatten()
            {
                if entry.file_type().is_some_and(|ft| ft.is_file()) {
                    files.insert(entry.path().to_string_lossy().into_owned());
                }
            }
        } else {
            not_found.push(path_str.clone());
        }
    }

    ExpansionResult {
        files: files.into_iter().collect(),
        not_found,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::Builder;

    fn setup_test_dir() -> (tempfile::TempDir, String) {
        let tmp_dir = Builder::new().prefix("test-expander").tempdir().unwrap();
        let root_path = tmp_dir.path().to_path_buf();

        // Standard files and directories
        fs::write(root_path.join("file1.txt"), "content1").unwrap();
        fs::create_dir(root_path.join("sub_dir")).unwrap();
        fs::write(root_path.join("sub_dir/file2.txt"), "content2").unwrap();
        fs::write(root_path.join("sub_dir/another_file.rs"), "content3").unwrap();
        fs::create_dir(root_path.join("empty_dir")).unwrap();

        // For ignore tests
        fs::write(root_path.join(".gitignore"), "*.log\nignored_dir/").unwrap();
        fs::write(root_path.join("a.log"), "log content").unwrap();
        fs::create_dir(root_path.join("ignored_dir")).unwrap();
        fs::write(
            root_path.join("ignored_dir/should_be_ignored.txt"),
            "ignored",
        )
        .unwrap();

        // For .git directory test
        fs::create_dir_all(root_path.join(".git/objects")).unwrap();
        fs::write(root_path.join(".git/config"), "git config").unwrap();

        (tmp_dir, root_path.to_str().unwrap().to_string())
    }

    #[test]
    fn test_mixed_paths() {
        let (_tmp_dir, root) = setup_test_dir();
        let paths = vec![
            format!("{root}/file1.txt"),
            format!("{root}/sub_dir"),
            "non_existent_file.txt".to_string(),
        ];

        let result = expand_and_validate(&paths, &[]);

        let expected_files = vec![
            format!("{root}/file1.txt"),
            format!("{root}/sub_dir/another_file.rs"),
            format!("{root}/sub_dir/file2.txt"),
        ];

        assert_eq!(
            result.files.into_iter().collect::<BTreeSet<_>>(),
            expected_files.into_iter().collect::<BTreeSet<_>>(),
        );
        assert_eq!(result.not_found, vec!["non_existent_file.txt".to_string()]);
    }

    #[test]
    fn test_files_only() {
        let (_tmp_dir, root) = setup_test_dir();
        let paths = vec![
            format!("{root}/file1.txt"),
            format!("{root}/sub_dir/file2.txt"),
        ];

        let result = expand_and_validate(&paths, &[]);

        let expected_files = paths.clone();

        assert_eq!(
            result.files.into_iter().collect::<BTreeSet<_>>(),
            expected_files.into_iter().collect::<BTreeSet<_>>(),
        );
        assert!(result.not_found.is_empty());
    }

    #[test]
    fn test_directory_only() {
        let (_tmp_dir, root) = setup_test_dir();
        let paths = vec![format!("{root}/sub_dir")];

        let result = expand_and_validate(&paths, &[]);

        let expected_files = vec![
            format!("{root}/sub_dir/file2.txt"),
            format!("{root}/sub_dir/another_file.rs"),
        ];

        assert_eq!(
            result.files.into_iter().collect::<BTreeSet<_>>(),
            expected_files.into_iter().collect::<BTreeSet<_>>(),
        );
        assert!(result.not_found.is_empty());
    }

    #[test]
    fn test_non_existent_paths() {
        let paths = vec!["no_such_file.rs".to_string(), "no_such_dir/".to_string()];
        let result = expand_and_validate(&paths, &[]);
        assert_eq!(
            result,
            ExpansionResult {
                files: vec![],
                not_found: paths,
            }
        );
    }

    #[test]
    fn test_deduplication() {
        let (_tmp_dir, root) = setup_test_dir();
        let paths = vec![
            format!("{root}/file1.txt"),
            format!("{root}/file1.txt"), // duplicate file
            format!("{root}/"),          // root dir which contains file1.txt
        ];

        let result = expand_and_validate(&paths, &[".git".to_string()]);

        let expected_files = vec![
            format!("{root}/.gitignore"),
            format!("{root}/file1.txt"),
            format!("{root}/sub_dir/file2.txt"),
            format!("{root}/sub_dir/another_file.rs"),
        ];

        assert_eq!(
            result.files.into_iter().collect::<BTreeSet<_>>(),
            expected_files.into_iter().collect::<BTreeSet<_>>(),
        );
        assert!(result.not_found.is_empty());
    }

    #[test]
    fn test_empty_input() {
        let paths: Vec<String> = vec![];
        let result = expand_and_validate(&paths, &[]);
        assert_eq!(
            result,
            ExpansionResult {
                files: vec![],
                not_found: vec![],
            }
        );
    }

    #[test]
    fn test_gitignore_is_respected() {
        let (_tmp_dir, root) = setup_test_dir();
        let paths = vec![format!("{root}/")];

        let result = expand_and_validate(&paths, &[".git".to_string()]);

        let unexpected_files = vec![
            format!("{root}/a.log"),
            format!("{root}/ignored_dir/should_be_ignored.txt"),
        ];

        for unexpected in unexpected_files {
            assert!(
                !result.files.contains(&unexpected),
                "File '{unexpected}' should have been ignored"
            );
        }

        let expected_files = vec![
            format!("{root}/.gitignore"),
            format!("{root}/file1.txt"),
            format!("{root}/sub_dir/another_file.rs"),
            format!("{root}/sub_dir/file2.txt"),
        ];

        assert_eq!(
            result.files.into_iter().collect::<BTreeSet<_>>(),
            expected_files.into_iter().collect::<BTreeSet<_>>(),
        );
    }

    #[test]
    fn test_git_directory_is_ignored() {
        let (_tmp_dir, root) = setup_test_dir();
        let paths = vec![format!("{root}/")];

        // This test now relies on the default config behavior
        let ignored_paths = vec![".git".to_string()];
        let result = expand_and_validate(&paths, &ignored_paths);
        let git_file = format!("{root}/.git/config");

        assert!(
            !result.files.contains(&git_file),
            ".git directory contents should be ignored"
        );
    }

    #[test]
    fn test_custom_ignored_paths() {
        let (_tmp_dir, root) = setup_test_dir();
        let paths = vec![format!("{root}/")];

        // Ignore the sub_dir entirely
        let ignored_paths = vec!["sub_dir".to_string()];
        let result = expand_and_validate(&paths, &ignored_paths);

        let unexpected_files = vec![
            format!("{root}/sub_dir/file2.txt"),
            format!("{root}/sub_dir/another_file.rs"),
        ];

        for unexpected in unexpected_files {
            assert!(
                !result.files.contains(&unexpected),
                "File '{unexpected}' should have been ignored via custom path"
            );
        }

        // Make sure other files are still there
        assert!(result.files.contains(&format!("{root}/file1.txt")));
    }
}
