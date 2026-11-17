//! NexSearch core: provider trait, result types and the ranking engine.
//!
//! Pure Rust logic, no Windows or Slint dependency.

use std::cmp::Ordering;
use std::path::PathBuf;

/// Built-in NexPanel commands that a search result can trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NexCommand {
    OpenSettings,
    Quit,
    ToggleAutostart,
}

/// What happens when a result is activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchAction {
    Launch(PathBuf),
    CopyText(String),
    Command(NexCommand),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// Higher is better.
    pub score: f32,
    pub action: SearchAction,
}

pub trait SearchProvider {
    fn query(&self, input: &str) -> Vec<SearchResult>;
}

/// Default maximum number of results returned by [`SearchEngine::search`].
pub const DEFAULT_LIMIT: usize = 8;

pub struct SearchEngine {
    providers: Vec<Box<dyn SearchProvider>>,
    limit: usize,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchEngine {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            limit: DEFAULT_LIMIT,
        }
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    pub fn add_provider(&mut self, provider: Box<dyn SearchProvider>) {
        self.providers.push(provider);
    }

    /// Queries all providers, sorts by score descending and truncates to the limit.
    ///
    /// Sorting is stable: equal scores keep provider order, then the order the
    /// provider returned them in. Blank input yields no results. NaN scores sort last.
    pub fn search(&self, input: &str) -> Vec<SearchResult> {
        let input = input.trim();
        if input.is_empty() {
            return Vec::new();
        }
        let mut all: Vec<SearchResult> =
            self.providers.iter().flat_map(|p| p.query(input)).collect();
        all.sort_by(|a, b| match (a.score.is_nan(), b.score.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal),
        });
        all.truncate(self.limit);
        all
    }
}

pub mod calc;
pub mod commands;
pub mod launcher;

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<(&'static str, f32)>);

    impl SearchProvider for Fixed {
        fn query(&self, _input: &str) -> Vec<SearchResult> {
            self.0
                .iter()
                .map(|(id, score)| SearchResult {
                    id: (*id).into(),
                    title: (*id).into(),
                    subtitle: String::new(),
                    score: *score,
                    action: SearchAction::CopyText((*id).into()),
                })
                .collect()
        }
    }

    fn ids(r: &[SearchResult]) -> Vec<&str> {
        r.iter().map(|x| x.id.as_str()).collect()
    }

    #[test]
    fn empty_engine_returns_nothing() {
        assert!(SearchEngine::new().search("x").is_empty());
    }

    #[test]
    fn blank_input_returns_nothing() {
        let mut e = SearchEngine::new();
        e.add_provider(Box::new(Fixed(vec![("a", 1.0)])));
        assert!(e.search("").is_empty());
        assert!(e.search("   ").is_empty());
    }

    #[test]
    fn sorts_by_score_descending() {
        let mut e = SearchEngine::new();
        e.add_provider(Box::new(Fixed(vec![
            ("low", 0.1),
            ("high", 0.9),
            ("mid", 0.5),
        ])));
        assert_eq!(ids(&e.search("q")), ["high", "mid", "low"]);
    }

    #[test]
    fn merges_multiple_providers() {
        let mut e = SearchEngine::new();
        e.add_provider(Box::new(Fixed(vec![("a1", 0.2), ("a2", 0.8)])));
        e.add_provider(Box::new(Fixed(vec![("b1", 0.5)])));
        assert_eq!(ids(&e.search("q")), ["a2", "b1", "a1"]);
    }

    #[test]
    fn stable_for_equal_scores() {
        let mut e = SearchEngine::new();
        e.add_provider(Box::new(Fixed(vec![("a1", 0.5), ("a2", 0.5)])));
        e.add_provider(Box::new(Fixed(vec![("b1", 0.5), ("b2", 0.5)])));
        assert_eq!(ids(&e.search("q")), ["a1", "a2", "b1", "b2"]);
    }

    #[test]
    fn limit_truncates_after_sorting() {
        let mut e = SearchEngine::new().with_limit(2);
        e.add_provider(Box::new(Fixed(vec![("a", 0.1), ("b", 0.9), ("c", 0.5)])));
        assert_eq!(ids(&e.search("q")), ["b", "c"]);
    }

    #[test]
    fn default_limit_applies() {
        let items: Vec<(&'static str, f32)> = vec![("x", 1.0); 20];
        let mut e = SearchEngine::new();
        e.add_provider(Box::new(Fixed(items)));
        assert_eq!(e.search("q").len(), DEFAULT_LIMIT);
    }

    #[test]
    fn zero_limit_returns_nothing() {
        let mut e = SearchEngine::new().with_limit(0);
        e.add_provider(Box::new(Fixed(vec![("a", 1.0)])));
        assert!(e.search("q").is_empty());
    }

    #[test]
    fn nan_scores_sort_last() {
        let mut e = SearchEngine::new();
        e.add_provider(Box::new(Fixed(vec![("nan", f32::NAN), ("ok", 0.1)])));
        assert_eq!(ids(&e.search("q")), ["ok", "nan"]);
    }

    #[test]
    fn provider_receives_trimmed_input() {
        struct Echo;
        impl SearchProvider for Echo {
            fn query(&self, input: &str) -> Vec<SearchResult> {
                vec![SearchResult {
                    id: input.into(),
                    title: input.into(),
                    subtitle: String::new(),
                    score: 1.0,
                    action: SearchAction::CopyText(input.into()),
                }]
            }
        }
        let mut e = SearchEngine::new();
        e.add_provider(Box::new(Echo));
        assert_eq!(e.search("  hi ")[0].id, "hi");
    }
}
