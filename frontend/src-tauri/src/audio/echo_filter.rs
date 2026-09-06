// audio/echo_filter.rs
//
// Fallback for setups where acoustic echo cancellation cannot run: drop a
// microphone segment whose text merely repeats what the system channel said a
// moment ago.
//
// This is deliberately off by default. It cannot tell an echo from a deliberate
// repetition, and repeating the other person's words back to them is a normal
// part of a conversation - so the acoustic canceller in `echo_cancel` is the
// primary mechanism, and this only helps when that one is unavailable.

use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::sync::Mutex;

/// How far back a system phrase may be and still explain a microphone segment.
const LOOKBACK_SECONDS: f64 = 8.0;
/// A system phrase may also start slightly after the microphone segment did,
/// since the two channels are transcribed independently.
const LOOKAHEAD_SECONDS: f64 = 2.0;
/// Short answers ("да", "угу") repeat naturally and are never treated as echo.
const MIN_CHARS: usize = 12;
/// Word overlap above which two texts count as the same phrase.
const SIMILARITY_THRESHOLD: f32 = 0.85;
/// Nothing older than this is worth keeping around.
const HISTORY_LIMIT: usize = 40;

struct SystemPhrase {
    start_time: f64,
    end_time: f64,
    words: Vec<String>,
}

/// Recent system-channel phrases, newest last.
#[derive(Default)]
pub struct EchoTextFilter {
    history: VecDeque<SystemPhrase>,
}

impl EchoTextFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.history.clear();
    }

    /// Remember a phrase the system channel produced.
    pub fn note_system_text(&mut self, start_time: f64, end_time: f64, text: &str) {
        let words = normalize(text);
        if words.is_empty() {
            return;
        }

        self.history.push_back(SystemPhrase {
            start_time,
            end_time,
            words,
        });
        while self.history.len() > HISTORY_LIMIT {
            self.history.pop_front();
        }
    }

    /// Does this microphone segment just repeat a recent system phrase?
    pub fn is_echo_of_recent_system(&self, start_time: f64, end_time: f64, text: &str) -> bool {
        if text.chars().count() < MIN_CHARS {
            return false;
        }
        let words = normalize(text);
        if words.is_empty() {
            return false;
        }

        self.history.iter().any(|phrase| {
            phrase.end_time >= start_time - LOOKBACK_SECONDS
                && phrase.start_time <= end_time + LOOKAHEAD_SECONDS
                && similarity(&words, &phrase.words) >= SIMILARITY_THRESHOLD
        })
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.history.len()
    }
}

static FILTER: Lazy<Mutex<EchoTextFilter>> = Lazy::new(|| Mutex::new(EchoTextFilter::new()));

/// Forget everything; called when a recording starts.
pub fn reset() {
    if let Ok(mut filter) = FILTER.lock() {
        filter.clear();
    }
}

pub fn note_system_text(start_time: f64, end_time: f64, text: &str) {
    if let Ok(mut filter) = FILTER.lock() {
        filter.note_system_text(start_time, end_time, text);
    }
}

pub fn is_echo_of_recent_system(start_time: f64, end_time: f64, text: &str) -> bool {
    match FILTER.lock() {
        Ok(filter) => filter.is_echo_of_recent_system(start_time, end_time, text),
        Err(_) => false,
    }
}

/// Lower-case words with punctuation stripped, so wording differences between
/// the two transcriptions of the same speech do not matter.
fn normalize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect()
}

/// Share of words the two phrases have in common, counting repeats
/// (multiset Jaccard). 1.0 means the same words in any order.
fn similarity(left: &[String], right: &[String]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let mut remaining: Vec<&String> = right.iter().collect();
    let mut shared = 0usize;
    for word in left {
        if let Some(position) = remaining.iter().position(|candidate| *candidate == word) {
            remaining.remove(position);
            shared += 1;
        }
    }

    let union = left.len() + right.len() - shared;
    shared as f32 / union as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_phrase(start: f64, end: f64, text: &str) -> EchoTextFilter {
        let mut filter = EchoTextFilter::new();
        filter.note_system_text(start, end, text);
        filter
    }

    #[test]
    fn identical_wording_within_the_window_is_echo() {
        let filter = with_phrase(10.0, 13.0, "Я хотел рассказать про вчерашний разговор.");
        assert!(filter.is_echo_of_recent_system(
            10.4,
            13.4,
            "я хотел рассказать про вчерашний разговор"
        ));
    }

    #[test]
    fn a_different_phrase_is_kept() {
        let filter = with_phrase(10.0, 13.0, "Я хотел рассказать про вчерашний разговор.");
        assert!(!filter.is_echo_of_recent_system(
            14.0,
            16.0,
            "И что вы почувствовали в тот момент?"
        ));
    }

    #[test]
    fn an_old_phrase_no_longer_explains_the_microphone() {
        let filter = with_phrase(1.0, 3.0, "Я хотел рассказать про вчерашний разговор.");
        assert!(!filter.is_echo_of_recent_system(
            40.0,
            43.0,
            "Я хотел рассказать про вчерашний разговор."
        ));
    }

    #[test]
    fn short_answers_are_never_treated_as_echo() {
        let filter = with_phrase(10.0, 11.0, "да, конечно");
        assert!(!filter.is_echo_of_recent_system(10.2, 11.2, "да, конечно"));
    }

    #[test]
    fn punctuation_and_case_do_not_matter() {
        let filter = with_phrase(5.0, 8.0, "мы говорили об этом на прошлой неделе подробно");
        assert!(filter.is_echo_of_recent_system(
            5.3,
            8.3,
            "Мы говорили об этом на прошлой неделе — подробно!"
        ));
    }

    /// One word out of twenty may differ between the two transcriptions.
    #[test]
    fn a_single_differing_word_in_a_long_phrase_still_matches() {
        let long = "мы очень долго говорили об этом на прошлой неделе и вы тогда сказали что вам стало заметно легче";
        let filter = with_phrase(5.0, 12.0, long);
        assert!(filter.is_echo_of_recent_system(5.2, 12.2, &long.replace("заметно", "немного")));
    }

    /// The local speaker paraphrasing what they just heard must survive: only a
    /// near-literal repetition counts as echo.
    #[test]
    fn a_paraphrase_is_kept() {
        let filter = with_phrase(5.0, 9.0, "мне было тяжело говорить об этом с родителями");
        assert!(!filter.is_echo_of_recent_system(
            9.5,
            12.0,
            "то есть вам было непросто обсуждать это дома"
        ));
    }

    #[test]
    fn history_does_not_grow_without_bound() {
        let mut filter = EchoTextFilter::new();
        for index in 0..(HISTORY_LIMIT + 20) {
            filter.note_system_text(index as f64, index as f64 + 1.0, "какая-то длинная фраза");
        }
        assert_eq!(filter.len(), HISTORY_LIMIT);
    }

    #[test]
    fn word_overlap_is_scored_as_a_multiset() {
        let left = normalize("один два три четыре");
        assert_eq!(similarity(&left, &left), 1.0);
        assert!(similarity(&left, &normalize("один два")) < 0.6);
        assert_eq!(similarity(&left, &normalize("")), 0.0);
    }
}
