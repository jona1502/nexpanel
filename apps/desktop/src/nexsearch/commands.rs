//! Command provider: NexPanel commands matched by prefix or fuzzy match on
//! German and English aliases.

use super::{NexCommand, SearchAction, SearchProvider, SearchResult};

struct Entry {
    command: NexCommand,
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
    /// Lowercase aliases (DE + EN).
    aliases: &'static [&'static str],
}

const ENTRIES: &[Entry] = &[
    Entry {
        command: NexCommand::OpenSettings,
        id: "cmd:settings",
        title: "Settings",
        subtitle: "Open NexPanel settings",
        aliases: &[
            "settings",
            "preferences",
            "options",
            "config",
            "einstellungen",
            "optionen",
            "konfiguration",
        ],
    },
    Entry {
        command: NexCommand::Quit,
        id: "cmd:quit",
        title: "Quit",
        subtitle: "Quit NexPanel",
        aliases: &[
            "quit",
            "exit",
            "close",
            "beenden",
            "schliessen",
            "schlie\u{00df}en",
        ],
    },
    Entry {
        command: NexCommand::ToggleAutostart,
        id: "cmd:autostart",
        title: "Autostart",
        subtitle: "Toggle start with Windows",
        aliases: &[
            "autostart",
            "startup",
            "start with windows",
            "windows-start",
            "systemstart",
        ],
    },
];

/// Max score a command can reach; stays below the calculator (100).
const EXACT: f32 = 90.0;
const PREFIX: f32 = 80.0;
const FUZZY: f32 = 40.0;

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut h = haystack.chars();
    needle.chars().all(|n| h.any(|c| c == n))
}

/// Scores `input` (already lowercased) against one alias.
fn score_alias(input: &str, alias: &str) -> Option<f32> {
    if alias == input {
        return Some(EXACT);
    }
    if alias.starts_with(input) {
        // Longer typed prefix = closer to the full alias.
        let coverage = input.chars().count() as f32 / alias.chars().count() as f32;
        return Some(PREFIX - 10.0 * (1.0 - coverage));
    }
    // Fuzzy only for 3+ chars to avoid noise from short inputs.
    if input.chars().count() >= 3 && is_subsequence(input, alias) {
        let coverage = input.chars().count() as f32 / alias.chars().count() as f32;
        return Some(FUZZY * coverage.max(0.25));
    }
    None
}

#[derive(Debug, Default)]
pub struct CommandProvider;

impl SearchProvider for CommandProvider {
    fn query(&self, input: &str) -> Vec<SearchResult> {
        let input = input.trim().to_lowercase();
        if input.is_empty() {
            return Vec::new();
        }
        ENTRIES
            .iter()
            .filter_map(|e| {
                let score = e
                    .aliases
                    .iter()
                    .filter_map(|a| score_alias(&input, a))
                    .fold(None, |best: Option<f32>, s| {
                        Some(best.map_or(s, |b| b.max(s)))
                    })?;
                Some(SearchResult {
                    id: e.id.into(),
                    title: e.title.into(),
                    subtitle: e.subtitle.into(),
                    score,
                    action: SearchAction::Command(e.command),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmds(input: &str) -> Vec<NexCommand> {
        let mut r = CommandProvider.query(input);
        r.sort_by(|a, b| b.score.total_cmp(&a.score));
        r.into_iter()
            .filter_map(|r| match r.action {
                SearchAction::Command(c) => Some(c),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_and_blank_yield_nothing() {
        assert!(CommandProvider.query("").is_empty());
        assert!(CommandProvider.query("   ").is_empty());
    }

    #[test]
    fn english_exact() {
        assert_eq!(cmds("settings"), [NexCommand::OpenSettings]);
        assert_eq!(cmds("quit"), [NexCommand::Quit]);
        assert_eq!(cmds("autostart"), [NexCommand::ToggleAutostart]);
    }

    #[test]
    fn german_aliases() {
        assert_eq!(cmds("einstellungen"), [NexCommand::OpenSettings]);
        assert_eq!(cmds("beenden"), [NexCommand::Quit]);
        assert_eq!(cmds("schlie\u{00df}en"), [NexCommand::Quit]);
        assert_eq!(cmds("systemstart"), [NexCommand::ToggleAutostart]);
    }

    #[test]
    fn prefix_match() {
        assert_eq!(cmds("sett")[0], NexCommand::OpenSettings);
        // "ein" also fuzzy-matches an autostart alias, but the prefix hit ranks first.
        assert_eq!(cmds("ein")[0], NexCommand::OpenSettings);
        assert_eq!(cmds("auto"), [NexCommand::ToggleAutostart]);
        assert_eq!(cmds("q"), [NexCommand::Quit]);
    }

    #[test]
    fn case_and_whitespace_insensitive() {
        assert_eq!(cmds("  SETTINGS "), [NexCommand::OpenSettings]);
        assert_eq!(cmds("Beenden"), [NexCommand::Quit]);
    }

    #[test]
    fn fuzzy_subsequence() {
        assert_eq!(cmds("stg"), [NexCommand::OpenSettings]);
        assert_eq!(cmds("atstrt"), [NexCommand::ToggleAutostart]);
    }

    #[test]
    fn fuzzy_needs_three_chars() {
        // "st" is a prefix of "startup" but "sg" (2 chars, non-prefix) must not fuzzy match.
        assert!(CommandProvider.query("sg").is_empty());
    }

    #[test]
    fn no_match() {
        assert!(CommandProvider.query("firefox").is_empty());
        assert!(CommandProvider.query("zzz").is_empty());
    }

    #[test]
    fn exact_beats_prefix_beats_fuzzy() {
        let exact = CommandProvider.query("quit")[0].score;
        let prefix = CommandProvider.query("qui")[0].score;
        let fuzzy = CommandProvider.query("qit")[0].score;
        assert!(exact > prefix, "{exact} > {prefix}");
        assert!(prefix > fuzzy, "{prefix} > {fuzzy}");
    }

    #[test]
    fn scores_stay_below_calculator() {
        for q in ["settings", "quit", "autostart", "stg"] {
            for r in CommandProvider.query(q) {
                assert!(r.score < 100.0);
            }
        }
    }

    #[test]
    fn one_result_per_command_with_best_alias() {
        // Several aliases may match one command; it must still appear once.
        let r = CommandProvider.query("st");
        let mut ids: Vec<_> = r.iter().map(|x| x.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), r.len());
    }

    #[test]
    fn result_shape() {
        let r = &CommandProvider.query("settings")[0];
        assert_eq!(r.id, "cmd:settings");
        assert_eq!(r.title, "Settings");
        assert_eq!(r.action, SearchAction::Command(NexCommand::OpenSettings));
    }

    #[test]
    fn works_inside_engine_with_calc_first() {
        use super::super::{SearchEngine, calc::CalcProvider};
        let mut e = SearchEngine::new();
        e.add_provider(Box::new(CommandProvider));
        e.add_provider(Box::new(CalcProvider));
        let r = e.search("2+2");
        assert_eq!(r[0].title, "4");
        let r = e.search("quit");
        assert_eq!(r[0].action, SearchAction::Command(NexCommand::Quit));
    }
}
