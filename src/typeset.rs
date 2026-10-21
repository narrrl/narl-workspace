//! ```typst blocks in markdown, typeset into SVG.
//!
//! Each block is a document of its own: its source is the only file there is,
//! and the fonts are the ones typst-assets builds into the binary. Nothing is
//! read from disk or fetched — an `#import "@preview/…"` or an `#image("…")`
//! is answered with an error in place of the block, as typst words it.
//!
//! The page fits what is on it and is transparent, and the text is the
//! theme's, so a block reads as part of the page it sits on. A block that
//! wants otherwise says so with its own `#set page(…)` or `#set text(…)`.

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::{LazyLock, Mutex},
};

use chrono::{Datelike, TimeDelta, Utc};
use typst::{
    diag::{FileError, FileResult, Severity, SourceDiagnostic},
    foundations::{Bytes, Datetime, Duration},
    layout::Abs,
    syntax::{FileId, Source, VirtualRoot},
    text::{Font, FontBook},
    utils::LazyHash,
    Library, LibraryExt, World, WorldExt,
};
use typst_layout::PagedDocument;

/// Set ahead of every block, on one line so an error's line number is the
/// block's own once one is taken off. `--fg` and the 14 px of the page's text.
const PREAMBLE: &str = "#set page(width: auto, height: auto, margin: 4pt, fill: none); \
    #set text(fill: rgb(\"#ccd2cb\"), size: 10.5pt)\n";

/// A block longer than this is not typeset: it is a document, not a block.
pub const MAX_SOURCE: usize = 64 * 1024;

/// How many blocks are kept typeset. Rendering the same page again, or the
/// preview while the rest of the document is written, asks for the same ones.
const CACHE_SIZE: usize = 256;

static LIBRARY: LazyLock<LazyHash<Library>> = LazyLock::new(|| LazyHash::new(Library::default()));

static FONTS: LazyLock<(LazyHash<FontBook>, Vec<Font>)> = LazyLock::new(|| {
    let fonts: Vec<Font> = typst_assets::fonts()
        .flat_map(|data| Font::iter(Bytes::new(data)))
        .collect();
    (LazyHash::new(FontBook::from_fonts(&fonts)), fonts)
});

static CACHE: LazyLock<Mutex<HashMap<u64, Typeset>>> = LazyLock::new(Default::default);

/// What a block became: its SVG, or what typst said about it.
pub type Typeset = Result<String, String>;

/// Typeset `source`, from the cache when it was typeset before. Blocking: the
/// layout runs on the calling thread, so call it from `spawn_blocking`.
pub fn render(source: &str) -> Typeset {
    let key = {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        hasher.finish()
    };
    if let Some(done) = CACHE.lock().unwrap().get(&key) {
        return done.clone();
    }
    let done = typeset(source);
    // comemo keeps what it memoised for every compile; let it go of what the
    // last few did not use, or it grows with every block ever written.
    typst::comemo::evict(10);
    let mut cache = CACHE.lock().unwrap();
    if cache.len() >= CACHE_SIZE {
        cache.clear();
    }
    cache.insert(key, done.clone());
    done
}

fn typeset(source: &str) -> Typeset {
    let block = Block {
        source: Source::detached(format!("{PREAMBLE}{source}")),
    };
    let document = typst::compile::<PagedDocument>(&block)
        .output
        .map_err(|errors| block.explain(&errors))?;
    Ok(typst_svg::svg_merged(
        &document,
        &Default::default(),
        Abs::pt(8.0),
    ))
}

/// The world a block is typeset in: its own source, and nothing else.
struct Block {
    source: Source,
}

impl Block {
    /// The errors as typst words them, each with the line of the block it is
    /// on and the hints it gives.
    fn explain(&self, errors: &[SourceDiagnostic]) -> String {
        let lines = self.source.lines();
        errors
            .iter()
            .filter(|error| error.severity == Severity::Error)
            .map(|error| {
                let line = self
                    .range(error.span)
                    .and_then(|range| lines.byte_to_line(range.start))
                    // The preamble is line 0, so the block's own count from 1.
                    .filter(|&line| line > 0);
                let mut said = match line {
                    Some(line) => format!("line {line}: {}", error.message),
                    None => error.message.to_string(),
                };
                for hint in &error.hints {
                    said.push_str(&format!("\n  hint: {}", hint.v));
                }
                said
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn only(&self, id: FileId) -> FileResult<()> {
        if id == self.source.id() {
            return Ok(());
        }
        Err(FileError::Other(Some(
            if matches!(id.root(), VirtualRoot::Package(_)) {
                "packages are not available here: a block is typeset with nothing but itself"
            } else {
                "files are not available here: a block is typeset with nothing but itself"
            }
            .into(),
        )))
    }
}

impl World for Block {
    fn library(&self) -> &LazyHash<Library> {
        &LIBRARY
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &FONTS.0
    }

    fn main(&self) -> FileId {
        self.source.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        self.only(id).map(|()| self.source.clone())
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.only(id)
            .map(|()| Bytes::from_string(self.source.text().to_owned()))
    }

    fn font(&self, index: usize) -> Option<Font> {
        FONTS.1.get(index).cloned()
    }

    /// The day in UTC, moved by the hours asked for, or by none: the server's
    /// zone is not the reader's, and `datetime.today()` is rarely what a block
    /// is for.
    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        let shift = offset.map_or(0, |offset| offset.seconds() as i64);
        let now = Utc::now() + TimeDelta::seconds(shift);
        Datetime::from_ymd(now.year(), now.month() as u8, now.day() as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn math_comes_out_as_svg() {
        let svg = render("$ sum_(k=1)^n k = (n(n+1)) / 2 $").unwrap();
        assert!(svg.starts_with("<svg"), "{svg}");
        // Glyphs are drawn as paths, in the theme's text colour.
        assert!(svg.contains("#ccd2cb"), "{svg}");
    }

    #[test]
    fn an_error_says_the_line_of_the_block() {
        let error = render("fine\n#let x = (\nmore").unwrap_err();
        assert!(error.starts_with("line 2: "), "{error}");
    }

    #[test]
    fn nothing_outside_the_block_is_read() {
        let error = render("#import \"@preview/cetz:0.3.0\"").unwrap_err();
        assert!(error.contains("packages are not available"), "{error}");
        let error = render("#image(\"/etc/passwd\")").unwrap_err();
        assert!(error.contains("files are not available"), "{error}");
    }

    #[test]
    fn a_loop_without_end_is_stopped() {
        assert!(render("#while true {}").is_err());
    }
}
