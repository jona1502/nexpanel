//! App launcher provider: indexes Start menu shortcuts in a background thread
//! and matches them by name prefix, word start or fuzzy subsequence.
//!
//! The `.lnk` file itself is launched (ShellExecute handles it), so no COM
//! shortcut resolution is needed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use super::{SearchAction, SearchProvider, SearchResult};

/// One launchable application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    pub path: PathBuf,
}

/// Source of application entries; mockable for tests.
pub trait AppSource: Send + Sync + 'static {
    /// May block (file system walk). Never call on the UI thread.
    fn scan(&self) -> Vec<AppEntry>;
}

/// Scans Start menu `.lnk` files below the given roots recursively.
pub struct StartMenuSource {
    roots: Vec<PathBuf>,
}

impl StartMenuSource {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    /// `%ProgramData%` and `%APPDATA%` Start menu program folders.
    pub fn from_env() -> Self {
        let rel = Path::new("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        let roots = ["ProgramData", "APPDATA"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|base| PathBuf::from(base).join(&rel))
            .collect();
        Self { roots }
    }
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<AppEntry>) {
    // Guard against pathological nesting / junction loops.
    if depth > 8 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk(&path, depth + 1, out);
            continue;
        }
        let is_lnk = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("lnk"));
        let name = path.file_stem().and_then(|s| s.to_str()).map(String::from);
        if let (true, Some(name)) = (is_lnk, name) {
            out.push(AppEntry { name, path });
        }
    }
}

impl AppSource for StartMenuSource {
    fn scan(&self) -> Vec<AppEntry> {
        let mut out = Vec::new();
        for root in &self.roots {
            walk(root, 0, &mut out);
        }
        out
    }
}

/// Max score of the launcher; stays below commands (<= 90) and calculator (100).
const EXACT: f32 = 85.0;
const PREFIX: f32 = 75.0;
const WORD: f32 = 60.0;
const FUZZY: f32 = 30.0;

struct Indexed {
    entry: AppEntry,
    lower: String,
}

fn is_uninstaller(lower: &str) -> bool {
    lower.contains("uninstall") || lower.contains("deinstall")
}

/// Filters uninstallers and removes duplicates by (case-insensitive) name.
/// First occurrence wins.
fn build_index(entries: Vec<AppEntry>) -> Vec<Indexed> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for entry in entries {
        let lower = entry.name.trim().to_lowercase();
        if lower.is_empty() || is_uninstaller(&lower) || !seen.insert(lower.clone()) {
            continue;
        }
        out.push(Indexed { entry, lower });
    }
    out
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut h = haystack.chars();
    needle.chars().all(|n| h.any(|c| c == n))
}

fn is_sep(c: char) -> bool {
    !c.is_alphanumeric()
}

fn word_start_match(input: &str, name: &str) -> bool {
    let mut prev_sep = true;
    for (i, c) in name.char_indices() {
        if prev_sep && !is_sep(c) && name[i..].starts_with(input) {
            return true;
        }
        prev_sep = is_sep(c);
    }
    false
}

/// Scores lowercase `input` against lowercase `name`.
fn score(input: &str, name: &str) -> Option<f32> {
    let coverage = input.chars().count() as f32 / name.chars().count().max(1) as f32;
    if name == input {
        return Some(EXACT);
    }
    if name.starts_with(input) {
        return Some(PREFIX - 10.0 * (1.0 - coverage));
    }
    if word_start_match(input, name) {
        return Some(WORD - 10.0 * (1.0 - coverage));
    }
    // Fuzzy only for 3+ chars to avoid noise.
    if input.chars().count() >= 3 && is_subsequence(input, name) {
        return Some(FUZZY * coverage.max(0.25));
    }
    None
}

pub struct LauncherProvider {
    index: Arc<OnceLock<Vec<Indexed>>>,
}

impl LauncherProvider {
    /// Starts indexing `source` on a background thread. Until it finishes,
    /// [`SearchProvider::query`] returns an empty list.
    pub fn spawn(source: impl AppSource) -> Self {
        let index = Arc::new(OnceLock::new());
        let slot = Arc::clone(&index);
        // If thread creation fails the index stays empty; no panic.
        let _ = std::thread::Builder::new()
            .name("nexsearch-index".into())
            .spawn(move || {
                let _ = slot.set(build_index(source.scan()));
            });
        Self { index }
    }

    /// Start menu launcher for the current user.
    pub fn start_menu() -> Self {
        Self::spawn(StartMenuSource::from_env())
    }

    /// Whether the background index is ready.
    pub fn is_ready(&self) -> bool {
        self.index.get().is_some()
    }

    #[cfg(test)]
    fn from_entries(entries: Vec<AppEntry>) -> Self {
        let index = Arc::new(OnceLock::new());
        let _ = index.set(build_index(entries));
        Self { index }
    }
}

impl SearchProvider for LauncherProvider {
    fn query(&self, input: &str) -> Vec<SearchResult> {
        let Some(index) = self.index.get() else {
            return Vec::new();
        };
        let input = input.trim().to_lowercase();
        if input.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<SearchResult> = index
            .iter()
            .filter_map(|i| {
                let s = score(&input, &i.lower)?;
                Some(SearchResult {
                    id: format!("app:{}", i.lower),
                    title: i.entry.name.clone(),
                    subtitle: "Application".into(),
                    score: s,
                    action: SearchAction::Launch(i.entry.path.clone()),
                })
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.title.len().cmp(&b.title.len()))
        });
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> AppEntry {
        AppEntry {
            name: name.into(),
            path: PathBuf::from(format!("C:\\{name}.lnk")),
        }
    }

    fn provider(names: &[&str]) -> LauncherProvider {
        LauncherProvider::from_entries(names.iter().map(|n| app(n)).collect())
    }

    fn titles(r: &[SearchResult]) -> Vec<&str> {
        r.iter().map(|x| x.title.as_str()).collect()
    }

    #[test]
    fn prefix_beats_word_start_beats_fuzzy() {
        let p = provider(&[
            "My Code Editor",
            "Visual Studio Code",
            "Codex",
            "Cool Dev Extra",
        ]);
        let r = p.query("code");
        assert_eq!(titles(&r)[0], "Codex");
        assert!(titles(&r).contains(&"My Code Editor"));
        let fuzzy = p.query("cde");
        assert!(titles(&fuzzy).contains(&"Cool Dev Extra"));
        let pre = r.iter().find(|x| x.title == "Codex").unwrap().score;
        let word = r
            .iter()
            .find(|x| x.title == "Visual Studio Code")
            .unwrap()
            .score;
        let fz = fuzzy
            .iter()
            .find(|x| x.title == "Cool Dev Extra")
            .unwrap()
            .score;
        assert!(pre > word && word > fz);
    }

    #[test]
    fn scores_stay_below_commands() {
        let p = provider(&["Firefox"]);
        assert!(p.query("firefox")[0].score <= 85.0);
        assert!(p.query("fire")[0].score < 85.0);
    }

    #[test]
    fn case_insensitive_and_launch_action() {
        let p = provider(&["Firefox"]);
        let r = p.query("FIRE");
        assert_eq!(r.len(), 1);
        assert_eq!(
            r[0].action,
            SearchAction::Launch(PathBuf::from("C:\\Firefox.lnk"))
        );
    }

    #[test]
    fn dedups_by_name() {
        let mut a = app("Firefox");
        a.path = PathBuf::from("C:\\one.lnk");
        let mut b = app("firefox");
        b.path = PathBuf::from("C:\\two.lnk");
        let p = LauncherProvider::from_entries(vec![a, b]);
        let r = p.query("fire");
        assert_eq!(r.len(), 1);
        assert_eq!(
            r[0].action,
            SearchAction::Launch(PathBuf::from("C:\\one.lnk"))
        );
    }

    #[test]
    fn filters_uninstallers() {
        let p = provider(&[
            "Uninstall Foo",
            "Foo Uninstaller",
            "Foo Deinstallation",
            "Foo",
        ]);
        assert_eq!(titles(&p.query("foo")), ["Foo"]);
        assert!(p.query("uninstall").is_empty());
    }

    #[test]
    fn no_match_and_blank() {
        let p = provider(&["Firefox"]);
        assert!(p.query("zzz").is_empty());
        assert!(p.query("  ").is_empty());
    }

    #[test]
    fn short_input_has_no_fuzzy() {
        let p = provider(&["Firefox"]);
        assert!(p.query("fx").is_empty());
        assert_eq!(p.query("fox").len(), 1);
    }

    #[test]
    fn empty_until_index_ready() {
        struct Gate(std::sync::Mutex<std::sync::mpsc::Receiver<()>>);
        impl AppSource for Gate {
            fn scan(&self) -> Vec<AppEntry> {
                let _ = self.0.lock().unwrap().recv();
                vec![app("Firefox")]
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let p = LauncherProvider::spawn(Gate(std::sync::Mutex::new(rx)));
        assert!(!p.is_ready());
        assert!(p.query("fire").is_empty());
        tx.send(()).unwrap();
        for _ in 0..200 {
            if p.is_ready() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(p.query("fire").len(), 1);
    }

    #[test]
    fn start_menu_source_finds_lnk_recursively() {
        let dir = std::env::temp_dir().join(format!("nexsearch-launcher-{}", std::process::id()));
        let sub = dir.join("Sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("A.lnk"), b"").unwrap();
        std::fs::write(sub.join("B.LNK"), b"").unwrap();
        std::fs::write(sub.join("c.txt"), b"").unwrap();
        let mut names: Vec<String> = StartMenuSource::new(vec![dir.clone()])
            .scan()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(names, ["A", "B"]);
    }
}
