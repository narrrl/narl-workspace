//! `.workspace.json`: what an export knows that its plain files cannot say.
//!
//! The archive is meant to read without this application, so it holds
//! markdown and files and nothing else in them. What a directory of files has
//! no place for — when a diary entry was written, a folder's name as typed
//! rather than as its slug, which space is the diary, the boards — goes into
//! this one file beside them. An import that finds it puts all of that back;
//! one that does not still gets the tree.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = ".workspace.json";

/// Bumped when a change would make an older import misread a newer file.
/// An import refuses a format it does not know rather than guessing.
pub const FORMAT: u32 = 1;

/// Where embedded files go in the archive, beside the spaces.
pub const MEDIA_DIR: &str = "media";

#[derive(Serialize, Deserialize, Default)]
pub struct Manifest {
    pub format: u32,
    /// Keyed by archive path: `work/notes` for a folder, `work/notes/a.md` for
    /// a document, the file's own path for a file, `work` for a space.
    #[serde(default)]
    pub nodes: BTreeMap<String, NodeMeta>,
    /// Keyed by the space's directory.
    #[serde(default)]
    pub spaces: BTreeMap<String, SpaceMeta>,
    /// Embedded files under `media/`, keyed by archive path, with the name
    /// they were uploaded under.
    #[serde(default)]
    pub media: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct NodeMeta {
    /// `space`, `folder`, `document` or `file`.
    pub kind: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SpaceMeta {
    pub has_board: bool,
    #[serde(default)]
    pub role: Option<String>,
    /// First to last; the first is the inbox and the last is done.
    #[serde(default)]
    pub lists: Vec<ListMeta>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ListMeta {
    pub name: String,
    #[serde(default)]
    pub cards: Vec<CardMeta>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CardMeta {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub due_at: Option<i64>,
    #[serde(default)]
    pub done_at: Option<i64>,
    /// The rule a card comes back by, `weekly` or `+2w`. Added without a
    /// bump of `FORMAT`: an older build ignores the field and imports the card
    /// as one that does not repeat, which loses the rule but misreads nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// The archive path of the document that holds the card's notes.
    #[serde(default)]
    pub document: Option<String>,
}
