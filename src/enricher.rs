use fancy_regex::Regex;
use once_cell::sync::Lazy;

#[derive(Debug, PartialEq)]
pub struct MessageEnrichments {
    pub mentioned_files: Vec<String>,
}

static AT_MENTION_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?<!\w)@([\w/\.\-]+)").expect("Invalid regex"));

pub fn extract_enrichments(content: &str) -> MessageEnrichments {
    let mentioned_files = AT_MENTION_REGEX
        .captures_iter(content)
        .filter_map(Result::ok)
        .map(|cap| cap[1].to_string())
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
