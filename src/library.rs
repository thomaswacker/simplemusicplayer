//! Directory listing: sub folders and playable audio files.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// File extensions we can decode (rodio/symphonia, plus libopus for Opus).
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "wav", "m4a", "mp4", "aif", "aiff",
];

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Default, Clone)]
pub struct Listing {
    pub dirs: Vec<Entry>,
    pub tracks: Vec<Entry>,
}

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

/// Lists a directory. Hidden entries are skipped; both groups are sorted naturally
/// ("2 - Foo" before "10 - Bar").
pub fn list_dir(dir: &Path) -> std::io::Result<Listing> {
    let mut listing = Listing::default();
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        // `metadata` follows symlinks, so linked folders/files work too.
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            listing.dirs.push(Entry { name, path });
        } else if meta.is_file() && is_audio_file(&path) {
            listing.tracks.push(Entry { name, path });
        }
    }
    listing.dirs.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    listing.tracks.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    Ok(listing)
}

/// File name without extension, used as fallback title.
pub fn display_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Case-insensitive comparison that orders embedded numbers by value.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let na = take_number(&mut a);
                let nb = take_number(&mut b);
                // Compare by length of the digit string without leading zeros, then lexically.
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(ca), Some(cb)) => {
                let ord = ca.to_lowercase().cmp(cb.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied().filter(|c| c.is_ascii_digit()) {
        s.push(c);
        it.next();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec![
            "10 - b", "2 - a", "1 - c", "Abc", "abd", "track 02", "track 1",
        ];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            v,
            [
                "1 - c", "2 - a", "10 - b", "Abc", "abd", "track 1", "track 02"
            ]
        );
    }

    #[test]
    fn audio_extensions() {
        assert!(is_audio_file(Path::new("/x/a.MP3")));
        assert!(is_audio_file(Path::new("a.flac")));
        assert!(!is_audio_file(Path::new("cover.jpg")));
        assert!(!is_audio_file(Path::new("noext")));
    }
}
