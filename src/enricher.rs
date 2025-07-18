use fancy_regex::Regex;
use once_cell::sync::Lazy;

#[derive(Debug, PartialEq)]
pub struct MessageEnrichments {
    pub mentioned_files: Vec<String>,
}

// This regex uses a negative lookbehind `(?<!\w)` to ensure the '@' is not preceded by a word character,
// which is the common case for email addresses.
// It then captures a file path that must end on a non-dot character, or it can be a single dot.
// This prevents the capture of trailing dots from sentence punctuation.
static AT_MENTION_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?<!\w)@((?:[\w/.-]*[\w/-])|\.)").expect("Invalid regex for @-mentions")
});

pub fn extract_enrichments(content: &str) -> MessageEnrichments {
    let mentioned_files = AT_MENTION_REGEX
        .captures_iter(content)
        .filter_map(Result::ok)
        .map(|cap| cap.get(1).unwrap().as_str().to_string())
        .collect();

    MessageEnrichments { mentioned_files }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_single_file() {
        let content = "hello, please read this file: @src/main.rs";
        let enrichments = extract_enrichments(content);
        assert_eq!(
            enrichments,
            MessageEnrichments {
                mentioned_files: vec!["src/main.rs".to_string()]
            }
        );
    }

    #[test]
    fn test_extract_multiple_files() {
        let content = "Check @src/main.rs and @Cargo.toml please";
        let enrichments = extract_enrichments(content);
        assert_eq!(
            enrichments,
            MessageEnrichments {
                mentioned_files: vec!["src/main.rs".to_string(), "Cargo.toml".to_string()]
            }
        );
    }

    #[test]
    fn test_no_mentioned_files() {
        let content = "hello world, how are you?";
        let enrichments = extract_enrichments(content);
        assert_eq!(
            enrichments,
            MessageEnrichments {
                mentioned_files: vec![]
            }
        );
    }

    #[test]
    fn test_inline_mention() {
        let content = "I saw this in (@src/cli.rs), which was interesting.";
        let enrichments = extract_enrichments(content);
        assert_eq!(
            enrichments,
            MessageEnrichments {
                mentioned_files: vec!["src/cli.rs".to_string()]
            }
        );
    }

    #[test]
    fn test_empty_content() {
        let content = "";
        let enrichments = extract_enrichments(content);
        assert_eq!(
            enrichments,
            MessageEnrichments {
                mentioned_files: vec![]
            }
        );
    }

    #[test]
    fn test_email_address_edge_case() {
        let content = "My email is test@example.com";
        let enrichments = extract_enrichments(content);
        assert_eq!(
            enrichments,
            MessageEnrichments {
                mentioned_files: vec![]
            }
        );
    }
}
