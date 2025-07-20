use serde::Deserialize;

/// Defines the elemental operations that can be part of a patch.
///
/// ### Reasoning
/// The operations are designed to be simple for an LLM to generate.
/// - `ReplaceRange` is a powerful primitive that handles modification, insertion, and deletion of
///   contiguous blocks of lines.
/// - `Insert` is a separate, more explicit operation for purely additive changes.
/// - The compact array format `["op_code", ...]` is token-efficient.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum PatchOperation {
    /// Replaces a contiguous range of lines with new content.
    #[serde(rename = "r")]
    Replace(ReplaceOperation),
    /// Inserts new lines after a specific existing line.
    #[serde(rename = "i")]
    Insert(InsertOperation),
}

/// Represents the arguments for a 'replace' operation.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceOperation {
    pub start_lid: String,
    pub end_lid: String,
    pub content: Vec<String>,
    pub context_before: Option<String>,
    pub context_after: Option<String>,
}

/// Represents the arguments for an 'insert' operation.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertOperation {
    pub after_lid: String,
    pub content: Vec<String>,
    pub context_before: Option<String>,
    pub context_after: Option<String>,
}
