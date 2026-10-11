//! **The screen.** Numbered, terse, and coloured so it can be scanned rather
//! than read.
//!
//! The maintainer's rules, ruled 2026-09-01 and restated because they are easy
//! to drift from (.rulings/ R-341): be brief and to the point; humans scan and
//! catch things by color; and nobody wants to read long captions and titles, or
//! notes that preach about data, authority and the design record.
//!
//! So: no citations, no section marks, no sentences. A label is one or two
//! words and a value is a value. **Prose lives behind `?` and nowhere else.**
//!
//! This is the exact opposite of the rule the rest of this repository follows,
//! and that is not a contradiction — the record is read once by somebody
//! deciding, and a screen is read every time by somebody doing.

/// Three states, and nothing else gets a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Untouched. The default is doing its job.
    Default,
    /// The operator changed it.
    Changed,
    /// It will not write.
    Invalid,
}

impl State {
    /// The SGR parameters for this state.
    ///
    /// **Dim rather than a colour for the default**, because a screen where
    /// everything is coloured is a screen where nothing is.
    const fn sgr(self) -> &'static str {
        match self {
            Self::Default => "2",
            Self::Changed => "32",
            Self::Invalid => "31",
        }
    }
}

/// Whether to emit colour at all.
///
/// **Redirected output gets none.** A `.toml` full of escape sequences is a
/// file nothing can read, and somebody will pipe this eventually.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour(pub bool);

impl Colour {
    /// Wrap `text` for `state`.
    #[must_use]
    pub fn paint(self, state: State, text: &str) -> String {
        if !self.0 {
            return text.to_owned();
        }
        format!("\x1b[{}m{text}\x1b[0m", state.sgr())
    }

    /// Dim, for chrome that is present and not being read.
    #[must_use]
    pub fn dim(self, text: &str) -> String {
        self.paint(State::Default, text)
    }
}

/// One line of the menu.
pub struct Row<'a> {
    /// What the operator types to edit it.
    pub number: usize,
    /// One or two words.
    pub label: &'a str,
    /// What it is set to.
    pub value: &'a str,
    /// How it got that way.
    pub state: State,
}

/// Render the whole screen.
///
/// Returned rather than printed so a test can read it, which is the only reason
/// this is a function and not a sequence of `println!`.
#[must_use]
pub fn render(rows: &[Row<'_>], footer: &[&str], blocked: Option<&str>, colour: Colour) -> String {
    let mut out = String::new();
    // The widest label, so the values line up in a column the eye can run down.
    let width = rows.iter().map(|r| r.label.len()).max().unwrap_or(0);
    // The numbers right-aligned, so a tenth row does not push its label one
    // column out of line with the nine above it.
    let digits = rows.iter().map(|r| r.number.to_string().len()).max().unwrap_or(1);
    for row in rows {
        out.push_str(&format!(
            "  {}  {:width$}  {}\n",
            colour.dim(&format!("{:>digits$}", row.number)),
            row.label,
            colour.paint(row.state, row.value),
        ));
    }
    out.push('\n');
    out.push_str("  ");
    out.push_str(&footer.iter().map(|f| colour.dim(f)).collect::<Vec<_>>().join("   "));
    if let Some(why) = blocked {
        out.push_str("   ");
        out.push_str(&colour.paint(State::Invalid, why));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **Redirected output carries no escapes.** Somebody will pipe this.
    #[test]
    fn colour_off_is_plain_text() {
        let rows = [Row {
            number: 1,
            label: "Hostname",
            value: "rivet-401ac",
            state: State::Changed,
        }];
        let text = render(&rows, &["w write"], None, Colour(false));
        assert!(!text.contains('\x1b'), "{text}");
        assert!(text.contains("rivet-401ac"));
    }

    /// The three states are three different colours, or the whole design is
    /// decoration.
    #[test]
    fn each_state_paints_differently() {
        let c = Colour(true);
        let a = c.paint(State::Default, "x");
        let b = c.paint(State::Changed, "x");
        let d = c.paint(State::Invalid, "x");
        assert_ne!(a, b);
        assert_ne!(b, d);
        assert_ne!(a, d);
    }

    /// Values line up under each other whatever the labels are, because a
    /// column is what makes a screen scannable.
    #[test]
    fn values_are_column_aligned() {
        let rows = [
            Row { number: 1, label: "Hostname", value: "a", state: State::Default },
            Row { number: 2, label: "Mode", value: "b", state: State::Default },
        ];
        let text = render(&rows, &[], None, Colour(false));
        let at: Vec<usize> = text
            .lines()
            .filter(|l| l.contains('a') || l.contains('b'))
            .filter_map(|l| l.rfind(char::is_alphanumeric))
            .collect();
        assert_eq!(at.len(), 2);
        assert_eq!(at[0], at[1], "{text}");
    }
}
