//! # mirsam-fonts
//!
//! Which file on this machine draws the typeface a document names.
//!
//! This is the adapter behind [`mirsam_core::ports::FontSource`], and it
//! exists because the question is not about Arabic. A paragraph names a
//! family — `Calibri`, `Traditional Arabic`, whatever theme or style supplied
//! it — and `mirsam-core` can say what that font would have to do with the
//! text, but not where the file is, or whether the machine has one at all.
//! Those are facts about the world, so the I/O lives out here and invariant 1
//! holds unchanged.
//!
//! ## What an answer means, and what it does not
//!
//! A hit means *this* machine has a file that answers to that name. It is not
//! a claim about the reader's machine, and a report must never let it read as
//! one: text set in a font the reader lacks renders in whatever their
//! application substitutes, and no amount of checking here can see that. The
//! honest claim is the negative one — a font that is here and cannot draw the
//! text will not draw it anywhere, because a font's `cmap` travels with the
//! font.
//!
//! A miss is a real and reportable state rather than an error. The machine has
//! no such font, so the tool cannot say what the reader will see, and saying
//! *that* is the finding.
//!
//! ## Reading a few hundred files to learn a few hundred names
//!
//! The index is built once, lazily, and only when something asks for a font.
//! Each file gives up its family names from its naming table alone — see
//! [`sfnt`] — rather than being parsed whole, which is the difference between
//! reading a few hundred kilobytes and reading every outline on the machine.
//! The whole file is read only for the one font that actually answered.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use mirsam_core::error::Result;
use mirsam_core::ports::{FontFile, FontSource};

pub mod sfnt;

/// Extensions worth opening. Anything else in a font directory — a `.dfont`,
/// a licence, a cache — is skipped without being read.
const FONT_EXTENSIONS: [&str; 4] = ["ttf", "otf", "ttc", "otc"];

/// The file and face that answer to one family name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Located {
    path: PathBuf,
    index: u32,
    /// The family exactly as the file states it, capitals and all. The lookup
    /// key is folded; this is what a report should print.
    family: String,
    /// Whether this is the family's upright, unemphasised face. See
    /// [`index`](SystemFonts::index) for what it decides.
    regular: bool,
}

/// The fonts installed on this machine, indexed by family name.
///
/// Construct once and share: the index is built on first use and kept, so a
/// document asking about forty paragraphs walks the font directories once.
#[derive(Debug, Default)]
pub struct SystemFonts {
    dirs: Vec<PathBuf>,
    index: OnceLock<BTreeMap<String, Located>>,
}

impl SystemFonts {
    /// The font directories of the platform this is running on.
    pub fn new() -> Self {
        Self::in_dirs(font_dirs())
    }

    /// An explicit set of directories, searched in the order given.
    ///
    /// The seam a `--font-dir` flag hangs from, and what the tests use: a
    /// suite that indexed the developer's machine would assert whatever
    /// happened to be installed on it.
    pub fn in_dirs(dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            dirs: dirs.into_iter().collect(),
            index: OnceLock::new(),
        }
    }

    /// The directories this will search, in precedence order.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// Every family name this machine can answer to, folded for lookup.
    ///
    /// Ascending, and stable across runs: the index is a sorted map and the
    /// directory walk sorts its entries, so two runs on one machine agree
    /// about which file won a contested name.
    pub fn families(&self) -> impl Iterator<Item = &str> {
        self.index().keys().map(String::as_str)
    }

    /// Build the index, resolving the two ways a family name is contested.
    ///
    /// *Within a family, the regular face wins.* `Arial` is eleven files on
    /// macOS and every one of them states the family name; a document asking
    /// for `Arial` means the upright one. Taking whichever sorted first would
    /// answer with `Arial Bold Italic.ttf`, which on this machine has no
    /// Arabic at all while `Arial.ttf` has — so the wrong pick is not a
    /// cosmetic difference but a wrong finding.
    ///
    /// *Between directories, the first to answer wins*, in the order
    /// [`font_dirs`] lists them, and a regular face never loses to one found
    /// earlier because a later directory is lower precedence by construction.
    fn index(&self) -> &BTreeMap<String, Located> {
        self.index.get_or_init(|| {
            let mut index: BTreeMap<String, Located> = BTreeMap::new();
            for dir in &self.dirs {
                for path in font_files(dir) {
                    // A machine with one truncated font is not a machine with
                    // no fonts: a file that will not parse is skipped, never
                    // fatal.
                    let Ok(faces) = sfnt::faces(&path) else {
                        continue;
                    };
                    for face in faces {
                        let regular = face.is_regular();
                        for family in face.families {
                            let key = fold(&family);
                            let takes = match index.get(&key) {
                                None => true,
                                Some(held) => regular && !held.regular,
                            };
                            if takes {
                                index.insert(
                                    key,
                                    Located {
                                        path: path.clone(),
                                        index: face.index,
                                        family,
                                        regular,
                                    },
                                );
                            }
                        }
                    }
                }
            }
            index
        })
    }
}

impl FontSource for SystemFonts {
    /// The first directory that answers wins, in the order [`font_dirs`]
    /// lists them: a font the user installed takes precedence over one of the
    /// same name shipped with the system, which is the order the operating
    /// systems themselves resolve in.
    fn load(&self, family: &str) -> Result<Option<FontFile>> {
        let Some(located) = self.index().get(&fold(family)) else {
            return Ok(None);
        };
        Ok(Some(FontFile {
            data: fs::read(&located.path)?,
            index: located.index,
            path: located.path.display().to_string(),
            family: located.family.clone(),
        }))
    }
}

/// The lookup form of a family name.
///
/// Documents are inconsistent about case and about the spaces around a name —
/// `Arial`, `arial`, ` Arial ` are one typeface — so the key is folded and the
/// name the file gave itself is kept separately for reports. Nothing else is
/// normalised: `Arial Narrow` is not `Arial`, and a source that guessed
/// otherwise would report a font the document never named.
fn fold(family: &str) -> String {
    family.trim().to_lowercase()
}

/// The font directories of the platform this is running on, most specific
/// first, and only the ones that exist.
///
/// User directories precede system ones because that is the precedence the
/// platforms give them: a font the user installed is the one their
/// application will use.
pub fn font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let under_home = |suffix: &str| home.as_ref().map(|h| h.join(suffix));

    let candidates: Vec<Option<PathBuf>> = if cfg!(target_os = "macos") {
        vec![
            under_home("Library/Fonts"),
            Some(PathBuf::from("/Library/Fonts")),
            Some(PathBuf::from("/System/Library/Fonts")),
        ]
    } else if cfg!(target_os = "windows") {
        vec![
            std::env::var_os("LOCALAPPDATA")
                .map(|d| PathBuf::from(d).join("Microsoft/Windows/Fonts")),
            std::env::var_os("WINDIR").map(|d| PathBuf::from(d).join("Fonts")),
        ]
    } else {
        vec![
            under_home(".local/share/fonts"),
            under_home(".fonts"),
            Some(PathBuf::from("/usr/local/share/fonts")),
            Some(PathBuf::from("/usr/share/fonts")),
        ]
    };

    candidates
        .into_iter()
        .flatten()
        .filter(|dir| dir.is_dir())
        .collect()
}

/// Every font file under `dir`, depth first, sorted at each level.
///
/// Sorted because the index is first-writer-wins and an unsorted directory
/// walk would let the filesystem decide which of two files claiming one family
/// answered for it. Directory *entries* decide recursion, so a symlinked
/// directory is not followed and a loop cannot be walked into.
fn font_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut here: Vec<(bool, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let is_dir = entry.file_type().ok()?.is_dir();
            Some((is_dir, entry.path()))
        })
        .collect();
    here.sort();

    let mut files = Vec::new();
    for (is_dir, path) in here {
        if is_dir {
            files.extend(font_files(&path));
        } else if is_font(&path) {
            files.push(path);
        }
    }
    files
}

fn is_font(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| FONT_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_family_is_matched_however_a_document_capitalised_it() {
        assert_eq!(fold(" Traditional Arabic "), "traditional arabic");
        assert_eq!(fold("ARIAL"), fold("arial"));
        // And no further: a narrower cut of a family is a different font.
        assert_ne!(fold("Arial Narrow"), fold("Arial"));
    }

    #[test]
    fn only_font_files_are_opened() {
        assert!(is_font(Path::new("/x/Arial.ttf")));
        assert!(is_font(Path::new("/x/Arial.TTC")));
        assert!(!is_font(Path::new("/x/fonts.dir")));
        assert!(!is_font(Path::new("/x/LICENSE")));
    }

    #[test]
    fn a_directory_that_is_not_there_is_empty_not_an_error() {
        assert!(font_files(Path::new("/no/such/directory")).is_empty());
        let source = SystemFonts::in_dirs([PathBuf::from("/no/such/directory")]);
        assert_eq!(source.families().count(), 0);
        assert_eq!(source.load("Arial").unwrap(), None);
    }
}
