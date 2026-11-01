//! Copy for AI: puts the open conversation (or the marked ones) on the clipboard as a prompt
//! for whatever AI tool you use, rather than the app talking to a model itself. Messages go
//! oldest first with who, when and to whom; quoted replies are dropped (the earlier messages
//! are there already) and blank runs squeezed, so the prompt stays short. Bodies not loaded yet
//! are fetched first; one that won't load goes in as its preview.

use rune::Tx;

use crate::app::App;
use crate::jmap::{Summary, format_addrs};

pub const DEFAULT_PROMPT: &str = "Summarize this email conversation: the key points, any decisions made, \
open questions, and anything I'm expected to do (with deadlines).";

impl App {
    /// The conversations Copy for AI takes: the marked ones, else the open one.
    fn ai_conversations(&self) -> Vec<Vec<Summary>> {
        let rows = self.targets();
        self.messages
            .iter()
            .filter(|m| rows.contains(&m.uid))
            .map(|row| self.thread(row).into_iter().cloned().collect())
            .collect()
    }

    pub fn copy_for_ai(&mut self, tx: &mut Tx) {
        let convs = self.ai_conversations();
        if convs.is_empty() {
            self.say("Select a conversation to copy", tx);
            return;
        }
        let missing: Vec<Summary> =
            convs.iter().flatten().filter(|m| !self.bodies.contains_key(&m.uid)).cloned().collect();
        self.ai_pending = Some(convs.iter().flatten().map(|m| m.uid.clone()).collect());
        if missing.is_empty() {
            self.finish_ai_copy(tx);
        } else {
            for m in &missing {
                self.body_failed.remove(&m.uid);
                self.load_body(m);
            }
            self.say(format!("Loading {} {} to copy…", missing.len(), if missing.len() == 1 { "message" } else { "messages" }), tx);
        }
    }

    /// Copies once every message waited for has loaded or failed; called as bodies arrive.
    pub fn finish_ai_copy(&mut self, tx: &mut Tx) {
        let Some(uids) = &self.ai_pending else { return };
        if uids.iter().any(|u| !self.bodies.contains_key(u) && !self.body_failed.contains_key(u)) {
            return;
        }
        let convs = self.ai_conversations();
        self.ai_pending = None;
        let count: usize = convs.iter().map(Vec::len).sum();
        let text = self.ai_prompt_text(&convs);
        tx.copy(text);
        let what = if convs.len() == 1 { "the conversation".to_owned() } else { format!("{} conversations", convs.len()) };
        self.say(format!("Copied {what} ({count} {}) for your AI", if count == 1 { "message" } else { "messages" }), tx);
    }

    fn ai_prompt_text(&self, convs: &[Vec<Summary>]) -> String {
        let prompt = self.settings.ai_prompt.trim();
        let mut out = String::from(if prompt.is_empty() { DEFAULT_PROMPT } else { prompt });
        for conv in convs {
            let subject = conv.last().map_or("", |m| m.subject.as_str());
            out.push_str(&format!("\n\n---\n\n# {}\n", if subject.is_empty() { "(no subject)" } else { subject }));
            for m in conv {
                let from = format_addrs(&m.from);
                out.push_str(&format!("\n## {} — {}\n", if from.is_empty() { "(no sender)" } else { &from }, crate::ui::dates::long(&m.received_at)));
                if !m.to.is_empty() {
                    out.push_str(&format!("To: {}\n", format_addrs(&m.to)));
                }
                match self.bodies.get(&m.uid) {
                    Some(b) => {
                        if !b.cc.is_empty() {
                            out.push_str(&format!("Cc: {}\n", format_addrs(&b.cc)));
                        }
                        out.push('\n');
                        out.push_str(&clean(&b.text));
                    }
                    None => out.push_str(&format!("\n(Couldn't load this message; its preview:) {}", m.preview)),
                }
                out.push('\n');
            }
        }
        out
    }
}

/// A body without its quoted replies ("> …" lines and the "On … wrote:" before them), with
/// runs of blank lines squeezed to one.
pub fn clean(body: &str) -> String {
    let lines: Vec<&str> = body.lines().map(str::trim_end).collect();
    let mut kept: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with('>') {
            continue;
        }
        // "On Tue, 28 Oct …, Someone wrote:" introducing a quote.
        let next_quoted = lines[i + 1..].iter().find(|l| !l.trim().is_empty()).is_some_and(|l| l.trim_start().starts_with('>'));
        if line.trim_end().ends_with("wrote:") && next_quoted {
            continue;
        }
        if line.trim().is_empty() && kept.last().is_none_or(|l| l.trim().is_empty()) {
            continue;
        }
        kept.push(line);
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    kept.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_quotes_and_squeezes_blank_lines() {
        let body = "Sounds good.\n\n\n\nThursday at 2.\n\nOn Wed, 28 Oct 2026, Ted wrote:\n\n> Does Thursday work?\n> Ted\n\nGrace\n";
        assert_eq!(clean(body), "Sounds good.\n\nThursday at 2.\n\nGrace");
    }

    #[test]
    fn copies_the_conversation_oldest_first_after_the_prompt() {
        crate::demo::test_config();
        let _one = crate::demo::test_app();
        let mut s = crate::app::init();
        crate::demo::thread_scene(&mut s);
        let text = s.ai_prompt_text(&s.ai_conversations());
        assert!(text.starts_with(DEFAULT_PROMPT));
        assert!(text.contains("# Re: Compiler meeting"));
        let first = text.find("A-0 system").expect("the oldest message");
        let last = text.find("bring the moth").expect("the newest");
        assert!(first < last, "oldest first");
        assert!(text.contains("## Grace Hopper <grace@example.com> — "));
        assert!(text.contains("To: Ted Martin <ted@fastmail.com>"));
    }

    #[test]
    fn keeps_a_wrote_line_that_isnt_a_quote() {
        assert_eq!(clean("She said she wrote:\nthe report"), "She said she wrote:\nthe report");
    }
}
