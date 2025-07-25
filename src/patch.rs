//! # Internal Patch Primitives
//!
//! This module defines the simple, internal "command" objects that the `FileState` module
//! executes. These structs are created by the `file_editor` module *after* all validation
//! (including anchor validation) has been performed.
//!
//! They contain the bare minimum information needed to mutate the file state and are
//! considered pre-validated and safe to execute by the `FileState`.

use fractional_index::FractionalIndex;

/// The new, simple, internal representation of a patch operation.
/// This is decoupled from the tool-facing request structs.
#[derive(Debug)]
pub enum PatchOperation {
    Insert(InsertOp),
    Replace(ReplaceOp),
}

/// A validated command to insert content.
#[derive(Debug)]
pub struct InsertOp {
    /// The LID of the line *after which* the new content should be inserted.
    /// If `None`, the content is inserted at the beginning of the file.
    pub after_lid: Option<FractionalIndex>,
    /// The new lines to insert, each with its own randomly generated suffix.
    pub content: Vec<(String, String)>,
}

/// A validated request to replace a range of lines.
#[derive(Debug, Clone)]
pub struct ReplaceOp {
    pub start_lid: FractionalIndex,
    pub end_lid: FractionalIndex,
    /// The new lines to replace the range with, each with its own randomly generated suffix.
    pub content: Vec<(String, String)>,
}
