use std::collections::BTreeSet;
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug, PartialEq, Eq)]
pub struct ExpansionResult {
    pub files: Vec<String>,
    pub not_found: Vec<String>,
}

pub fn expand_and_validate(paths: &[String]) -> ExpansionResult {
    let mut files = BTreeSet::new();
    let mut not_found = Vec::new();

    for path_str in paths {
        let path = Path::new(path_str);
        if path.exists() {
            if path.is_dir() {
                expand_directory(path, &mut files);
            } else if path.is_file() {
                files.insert(path.to_string_lossy().into_owned());
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

fn expand_directory(dir_path: &Path, files: &mut BTreeSet<String>) {
    for entry in WalkDir::new(dir_path).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            files.insert(entry.path().to_string_lossy().into_owned());
        }
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

        fs::write(root_path.join("file1.txt"), "content1").unwrap();
        fs::create_dir(root_path.join("sub_dir")).unwrap();
        fs::write(root_path.join("sub_dir/file2.txt"), "content2").unwrap();
        fs::write(root_path.join("sub_dir/another_file.rs"), "content3").unwrap();
        fs::create_dir(root_path.join("empty_dir")).unwrap();

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

        let mut result = expand_and_validate(&paths);
        result.files.sort(); // BTreeSet gives sorted output, but let's be explicit

        let mut expected_files = vec![
            format!("{root}/file1.txt"),
            format!("{root}/sub_dir/file2.txt"),
            format!("{root}/sub_dir/another_file.rs"),
        ];
        expected_files.sort();

        assert_eq!(
            result,
            ExpansionResult {
                files: expected_files,
                not_found: vec!["non_existent_file.txt".to_string()],
            }
        );
    }

    #[test]
    fn test_files_only() {
        let (_tmp_dir, root) = setup_test_dir();
        let paths = vec![
            format!("{root}/file1.txt"),
            format!("{root}/sub_dir/file2.txt"),
        ];

        let mut result = expand_and_validate(&paths);
        result.files.sort();

        let mut expected_files = paths.clone();
        expected_files.sort();

        assert_eq!(
            result,
            ExpansionResult {
                files: expected_files,
                not_found: vec![],
            }
        );
    }

    #[test]
    fn test_directory_only() {
        let (_tmp_dir, root) = setup_test_dir();
        let paths = vec![format!("{root}/sub_dir")];

        let mut result = expand_and_validate(&paths);
        result.files.sort();

        let mut expected_files = vec![
            format!("{root}/sub_dir/file2.txt"),
            format!("{root}/sub_dir/another_file.rs"),
        ];
        expected_files.sort();

        assert_eq!(
            result,
            ExpansionResult {
                files: expected_files,
                not_found: vec![],
            }
        );
    }

    #[test]
    fn test_non_existent_paths() {
        let paths = vec!["no_such_file.rs".to_string(), "no_such_dir/".to_string()];
        let result = expand_and_validate(&paths);
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

        let mut result = expand_and_validate(&paths);
        result.files.sort();

        let mut expected_files = vec![
            format!("{root}/file1.txt"),
            format!("{root}/sub_dir/file2.txt"),
            format!("{root}/sub_dir/another_file.rs"),
        ];
        expected_files.sort();

        assert_eq!(
            result,
            ExpansionResult {
                files: expected_files,
                not_found: vec![],
            }
        );
    }

    #[test]
    fn test_empty_input() {
        let paths: Vec<String> = vec![];
        let result = expand_and_validate(&paths);
        assert_eq!(
            result,
            ExpansionResult {
                files: vec![],
                not_found: vec![],
            }
        );
    }
}
