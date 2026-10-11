//! **The fleet screen** — the list of devices, and where the interface starts.
//!
//! Ruled by the maintainer on 2026-08-27: the device list is the correct
//! starting point, with a reachability indicator, and add / modify / hide
//! beside it (.rulings/ R-327).
//!
//! # Hide, not delete
//!
//! Also his: the screen's delete should probably not delete at all but hide the
//! device thoroughly — and it is the right instinct for a reason `ADR-025`
//! §4.1 already gives. A device's **identifier is authoritative** and a record
//! is the only place it exists; deleting the record loses the one fact that
//! cannot be reconstructed, and a later device of the same name would be a
//! *different* device wearing an old label.
//!
//! So this screen hides. `remove device` still deletes and stays where it is —
//! at the command line, typed on purpose, by somebody who has read what it
//! says.
//!
//! # Selection exists before anything uses it
//!
//! Editing several devices at once is **in scope and deferred**, ruled the same
//! day. Planning around it costs almost nothing if it is done now and a rewrite
//! if it is not, so [`Fleet::marked`] exists and `space` fills it while the only
//! thing that reads it is the count in the footer. See `crate::tui`'s header for
//! the other two halves of that.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::register::{Device, Register};
use crate::tui::reach::Reach;
use crate::tui::theme::{Glyphs, Palette};

/// What the fleet screen is showing.
pub struct Fleet {
    /// Which row the cursor is on, as an index into [`Fleet::visible`]'s answer.
    pub cursor: usize,
    /// Whether hidden devices are shown. `H` toggles it.
    pub show_hidden: bool,
    /// Devices marked for an operation on several at once — **deferred, and
    /// present so that it is not a rewrite later.**
    pub marked: Vec<String>,
}

impl Default for Fleet {
    fn default() -> Self {
        Self::new()
    }
}

impl Fleet {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cursor: 0,
            show_hidden: false,
            marked: Vec::new(),
        }
    }

    /// The devices this screen is showing, in the register's own order.
    ///
    /// **The register's order is by identifier** and is therefore stable across
    /// renames — a list that reordered itself when somebody edited a label
    /// would move the row under the cursor.
    #[must_use]
    pub fn visible<'a>(&self, register: &'a Register) -> Vec<&'a Device> {
        register
            .devices
            .values()
            .filter(|d| self.show_hidden || !d.hidden)
            .collect()
    }

    /// The device under the cursor, if there is one.
    #[must_use]
    pub fn selected<'a>(&self, register: &'a Register) -> Option<&'a Device> {
        self.visible(register).get(self.cursor).copied()
    }

    /// Move the cursor, wrapping.
    ///
    /// Wraps because a list that stops at the ends makes a person press a key
    /// and wonder whether it registered — `crate::tui`'s original browser had
    /// the same note and it was right.
    pub fn step(&mut self, by: isize, register: &Register) {
        let count = self.visible(register).len();
        if count == 0 {
            self.cursor = 0;
            return;
        }
        let here = isize::try_from(self.cursor).unwrap_or(0);
        let next = (here + by).rem_euclid(isize::try_from(count).unwrap_or(1));
        self.cursor = usize::try_from(next).unwrap_or(0);
    }

    /// Mark or unmark the device under the cursor.
    pub fn toggle_mark(&mut self, register: &Register) {
        let Some(id) = self.selected(register).map(|d| d.id.clone()) else {
            return;
        };
        if let Some(at) = self.marked.iter().position(|m| *m == id) {
            self.marked.remove(at);
        } else {
            self.marked.push(id);
        }
    }
}

/// One row's worth of what a reader needs.
///
/// **Assembled before it is drawn**, so the columns can be tested without a
/// terminal — which is the only way a layout gets asserted at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub role: String,
    pub location: String,
    pub reach: Reach,
    pub hidden: bool,
    pub marked: bool,
}

/// The rows, from the register and what the probes have answered.
#[must_use]
pub fn rows(
    fleet: &Fleet,
    register: &Register,
    reach: &std::collections::BTreeMap<String, Reach>,
) -> Vec<Row> {
    fleet
        .visible(register)
        .into_iter()
        .map(|d| Row {
            id: d.id.clone(),
            name: d.name.clone(),
            role: d.role.clone().unwrap_or_default(),
            location: d.location.clone().unwrap_or_default(),
            // **Unknown until something says otherwise** — `crate::tui::reach`
            // carries why that is a state and not a placeholder.
            reach: reach.get(&d.id).copied().unwrap_or(Reach::Unknown),
            hidden: d.hidden,
            marked: fleet.marked.contains(&d.id),
        })
        .collect()
}

/// Draw it.
pub fn draw(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    fleet: &Fleet,
    register: &Register,
    reach: &std::collections::BTreeMap<String, Reach>,
    glyphs: Glyphs,
    palette: Palette,
) {
    let rows = rows(fleet, register, reach);
    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);

    let title = format!(
        " Rivet Fleet — {} device{} ",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" }
    );

    let lines: Vec<Line> = if rows.is_empty() {
        // **An empty register is an answer**, and it says what to do next —
        // `crate::main`'s `list_devices` makes the same point at the command
        // line and this is the same sentence in the same situation.
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  This register has no devices.",
                palette.plain(),
            )),
            Line::from(Span::styled(
                "  Press `a` to enroll one, or run `rivet-fleet add device --name <name>`.",
                palette.faint(),
            )),
        ]
    } else {
        rows.iter()
            .enumerate()
            .map(|(at, row)| line_for(row, at == fleet.cursor, glyphs, palette))
            .collect()
    };

    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title)),
        chunks[0],
    );
    frame.render_widget(Paragraph::new(footer(fleet, glyphs, palette)), chunks[1]);
}

/// One device, as a line.
/// One column: exactly `width` characters, always with a gap before the next.
///
/// **`format!("{:<22}", ...)` was here and it is a minimum, not a maximum.** A
/// role of exactly 22 characters padded to 22 and the next column began in the
/// very next cell, so the fleet list rendered
///
/// > `development test boarddesk`
///
/// - a role and a location with nothing between them, which reads as one
/// nonsense word. Found by running the interface, 2026-08-27, `PLAN.md` 55.
///
/// So the text is cut to `width - 1` and the last cell is always a space. A cut
/// is marked with an ellipsis, because a value silently shortened to something
/// that looks complete is worse than one that says it is not:
/// `development test bo...` is honest and `development test bo` is a different
/// location.
fn column(text: &str, width: usize) -> String {
    let room = width.saturating_sub(1);
    let count = text.chars().count();
    if count <= room {
        let mut out = String::with_capacity(width);
        out.push_str(text);
        for _ in count..width {
            out.push(' ');
        }
        return out;
    }
    if room == 0 {
        // A column one cell wide is all gap. There is nowhere to put the
        // marker either, and a column that came back two cells wide would push
        // every column after it sideways — which is the defect this function
        // exists to prevent, arriving from the other direction.
        return " ".repeat(width);
    }
    // `room - 1` for the marker itself.
    let keep = room - 1;
    let mut out: String = text.chars().take(keep).collect();
    out.push('\u{2026}');
    out.push(' ');
    out
}

fn line_for(row: &Row, selected: bool, glyphs: Glyphs, palette: Palette) -> Line<'static> {
    let (glyph, style) = match row.reach {
        Reach::Up => (glyphs.up, palette.up()),
        Reach::Down => (glyphs.down, palette.down()),
        // **The warning glyph, not the down one.** `ADR-034` §6.1's neighbour:
        // a machine answering with a key this end does not pin is not a machine
        // that is off, and the two need different things done about them. The
        // word beside it says `STRANGER`, which is `crate::tui::theme`'s rule
        // that no glyph carries meaning alone.
        Reach::Stranger => (glyphs.warn, palette.warn()),
        Reach::Unknown => (glyphs.unknown, palette.unknown()),
        // **Faint, because this is a fact about THIS END.** A run with no
        // identity has not learned anything bad about the machine, and colouring
        // it as trouble would attribute a missing flag to the board.
        Reach::Unasked => (glyphs.unknown, palette.faint()),
    };

    let mut spans = vec![
        Span::styled(
            if selected {
                format!("{} ", glyphs.cursor)
            } else {
                "  ".to_string()
            },
            palette.plain(),
        ),
        Span::styled(if row.marked { "[*] " } else { "    " }, palette.plain()),
        // The glyph, and then the word — **never the glyph alone**, which is
        // `crate::tui::theme`'s whole fallback story.
        Span::styled(format!("{glyph} "), style),
        Span::styled(column(&row.name, 20), palette.plain()),
        Span::styled(column(row.reach.word(), 10), style),
        Span::styled(column(&row.role, 22), palette.faint()),
        Span::styled(column(&row.location, 14), palette.faint()),
    ];
    if row.hidden {
        spans.push(Span::styled("hidden", palette.faint()));
    }

    let line = Line::from(spans);
    if selected { line.style(palette.selected()) } else { line }
}

/// The key hints.
///
/// **Every key this screen answers to is on it.** A full-screen program whose
/// keys are not written down is one a person has to be told about, and being
/// told is not something a program can rely on.
fn footer(fleet: &Fleet, glyphs: Glyphs, palette: Palette) -> Line<'static> {
    let mut text = String::from(
        " [↑↓] move  [↵] open  [space] mark  [H]idden  [r]e-probe  [q]uit",
    );
    if !fleet.marked.is_empty() {
        // The count is shown because the marks are, and a person who marked
        // three devices and scrolled away should not have to count them again.
        // Nothing acts on it yet — see this module's header.
        text.push_str(&format!(
            "   {} {} marked (bulk edit is not built)",
            glyphs.pending,
            fleet.marked.len()
        ));
    }
    Line::from(Span::styled(text, palette.faint()))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use std::collections::BTreeMap;

    fn register(devices: &[(&str, &str, bool)]) -> Register {
        let mut register = Register::empty(std::path::PathBuf::from("/nowhere"));
        for (id, name, hidden) in devices {
            register.devices.insert(
                (*id).to_string(),
                Device {
                    id: (*id).to_string(),
                    name: (*name).to_string(),
                    hidden: *hidden,
                    ..Device::default()
                },
            );
        }
        register
    }

    /// **A machine that answered with the wrong key does not look like one that
    /// is switched off**, and neither of them looks like a run that has not
    /// asked. `ADR-034` §6.1's neighbour, at the place it is decided.
    ///
    /// Five states, five rows, and what is asserted is that the **style** for
    /// each is distinct where it has to be — because a person reads the colour
    /// before the word, and a `STRANGER` drawn as `down` would be an operator
    /// power-cycling a board that is answering perfectly well.
    #[test]
    fn each_reachability_is_drawn_as_its_own_thing() {
        let (glyphs, palette) = (Glyphs::ascii(), Palette::from_env());
        let style_of = |reach: Reach| {
            let row = Row {
                reach,
                ..a_row()
            };
            // The glyph span is the third: cursor, marker, then the glyph.
            let line = line_for(&row, false, glyphs, palette);
            (line.spans[2].content.to_string(), line.spans[2].style)
        };
        let up = style_of(Reach::Up);
        let down = style_of(Reach::Down);
        let stranger = style_of(Reach::Stranger);
        let unasked = style_of(Reach::Unasked);

        // The three that make a claim about the machine are three claims.
        assert_ne!(up.1, down.1, "up and down look the same");
        assert_ne!(stranger.1, down.1, "a stranger looks like a dead machine");
        assert_ne!(stranger.1, up.1, "a stranger looks like a healthy machine");
        assert_ne!(stranger.0, down.0, "a stranger draws the same glyph as down");

        // **And the one that is a fact about THIS END is not drawn as trouble.**
        // A missing --identity is not news about the board.
        assert_ne!(unasked.1, down.1, "no identity is drawn as a dead machine");
        assert_ne!(unasked.1, stranger.1, "no identity is drawn as a stranger");
    }

    fn a_row() -> Row {
        Row {
            id: "0011223344556677".to_string(),
            name: "kitchen-pi".to_string(),
            role: String::new(),
            location: String::new(),
            reach: Reach::Unknown,
            hidden: false,
            marked: false,
        }
    }

    /// **Hidden devices are out of the way and not gone**, which is the whole
    /// of the maintainer's instruction to hide a device thoroughly rather than
    /// delete it.
    #[test]
    fn a_hidden_device_is_absent_until_asked_for_and_never_removed() {
        let register = register(&[("a", "board", false), ("b", "retired", true)]);
        let mut fleet = Fleet::new();

        assert_eq!(fleet.visible(&register).len(), 1);
        fleet.show_hidden = true;
        assert_eq!(fleet.visible(&register).len(), 2);

        // And the register still holds both, whichever way the screen is set.
        assert_eq!(register.devices.len(), 2);
    }

    /// The cursor wraps, so a person pressing a key always sees something move.
    #[test]
    fn the_cursor_wraps_at_both_ends() {
        let register = register(&[("a", "one", false), ("b", "two", false)]);
        let mut fleet = Fleet::new();

        fleet.step(1, &register);
        assert_eq!(fleet.cursor, 1);
        fleet.step(1, &register);
        assert_eq!(fleet.cursor, 0, "did not wrap forwards");
        fleet.step(-1, &register);
        assert_eq!(fleet.cursor, 1, "did not wrap backwards");
    }

    /// An empty register does not move a cursor anywhere, and does not panic.
    #[test]
    fn an_empty_register_has_nowhere_to_move_and_says_so_rather_than_crashing() {
        let register = register(&[]);
        let mut fleet = Fleet::new();
        fleet.step(1, &register);
        fleet.step(-1, &register);
        assert_eq!(fleet.cursor, 0);
        assert!(fleet.selected(&register).is_none());
        assert!(rows(&fleet, &register, &BTreeMap::new()).is_empty());
    }

    /// **A device nobody has probed reads as unknown, never as down.**
    #[test]
    fn a_device_with_no_answer_yet_is_unknown_rather_than_down() {
        let register = register(&[("a", "board", false)]);
        let rows = rows(&Fleet::new(), &register, &BTreeMap::new());
        assert_eq!(rows[0].reach, Reach::Unknown);
        assert_eq!(rows[0].reach.word(), "checking");
    }

    /// An answer reaches the row it was about — by identifier, because a label
    /// can change while a probe is in flight.
    #[test]
    fn an_answer_lands_on_the_device_it_was_about() {
        let register = register(&[("a", "board", false), ("b", "other", false)]);
        let mut reach = BTreeMap::new();
        reach.insert("b".to_string(), Reach::Up);

        let rows = rows(&Fleet::new(), &register, &reach);
        let board = rows.iter().find(|r| r.name == "board").expect("board");
        let other = rows.iter().find(|r| r.name == "other").expect("other");
        assert_eq!(board.reach, Reach::Unknown);
        assert_eq!(other.reach, Reach::Up);
    }

    /// **Marking exists before anything reads it**, so bulk editing is a
    /// feature to add rather than a shape to change (ruled in scope, deferred,
    /// 2026-08-27).
    #[test]
    fn marks_are_kept_by_identifier_and_survive_the_cursor_moving() {
        let register = register(&[("a", "one", false), ("b", "two", false)]);
        let mut fleet = Fleet::new();

        fleet.toggle_mark(&register);
        assert_eq!(fleet.marked, ["a"]);
        fleet.step(1, &register);
        fleet.toggle_mark(&register);
        assert_eq!(fleet.marked, ["a", "b"]);

        // Unmarking is the same key.
        fleet.toggle_mark(&register);
        assert_eq!(fleet.marked, ["a"]);
    }

    /// The rows carry what the screen shows, so a column can be asserted
    /// without a terminal.
    #[test]
    fn a_row_carries_every_column_the_screen_shows() {
        let mut register = register(&[("a", "board", false)]);
        let device = register.devices.get_mut("a").expect("the device");
        device.role = Some("test board".to_string());
        device.location = Some("desk".to_string());

        let rows = rows(&Fleet::new(), &register, &BTreeMap::new());
        assert_eq!(rows[0].name, "board");
        assert_eq!(rows[0].role, "test board");
        assert_eq!(rows[0].location, "desk");
    }

    /// The defect, as a test: a role of **exactly** the column width used to
    /// touch the next column. `PLAN.md` 55.
    #[test]
    fn a_value_that_exactly_fills_its_column_still_leaves_a_gap() {
        let exact = "development test board";
        assert_eq!(exact.chars().count(), 22);
        let out = column(exact, 22);
        assert_eq!(out.chars().count(), 22);
        assert!(out.ends_with(' '), "{out:?}");
        // And what the screen actually renders, which is the thing that was
        // wrong: the two columns, side by side, with something between them.
        let row = format!("{}{}", column(exact, 22), column("desk", 14));
        assert!(!row.contains("boarddesk"), "{row:?}");
        assert!(row.starts_with("development test boa\u{2026} "), "{row:?}");
    }

    #[test]
    fn a_column_is_always_exactly_its_width() {
        for text in ["", "a", "desk", "development test board", &"z".repeat(80)] {
            for width in [1_usize, 2, 10, 14, 22] {
                let out = column(text, width);
                assert_eq!(out.chars().count(), width, "{text:?} at {width}");
                assert!(out.ends_with(' '), "{text:?} at {width}: {out:?}");
            }
        }
    }

    /// A cut says it was cut. A value silently shortened to something that
    /// looks complete is a different value wearing the right shape.
    #[test]
    fn a_cut_column_says_so() {
        assert!(column("development test board", 10).contains('\u{2026}'));
        assert!(!column("desk", 10).contains('\u{2026}'));
    }
}
