//! What the operator sees, as data rather than as escape sequences.
//!
//! [`render`] turns an editor into a [`Frame`]: a list of rows, each a list of
//! styled [`Span`]s, plus where the cursor goes. **No ANSI appears in this
//! module.** Turning a [`Style`] into `\x1b[7m` is `crate::host`'s, which means
//! every assertion about what is on screen is an assertion about text —
//! `assert!(frame.row(0).contains("VIEW"))` rather than a regex over escape
//! codes, and a rendering test that cannot be fooled by a colour changing.
//!
//! # The layout is `nano`'s, to the row
//!
//! ```text
//!   row 0          title bar        name, and Modified / VIEW
//!   rows 1..h-4    the text
//!   row h-3        status / prompt  the one line that talks back
//!   rows h-2, h-1  the key help     two lines, always visible
//! ```
//!
//! Four rows of chrome, exactly as `nano` spends them, and the two help lines
//! are **not** hidden to win back space. That is the single most `nano`-shaped
//! decision in this program: an editor whose keys are discoverable without
//! reading anything is the difference between a tool an operator can use at 2am
//! and one they have to learn first.
//!
//! **Every key in [`HELP_EDIT`] is one `nano` binds to the same thing.** That
//! is the whole design rule for the key map — if a `nano` user has to think, it
//! is wrong — and `every_advertised_key_is_handled` is what keeps the two lines
//! from advertising something `crate::editor` does not do.
//!
//! # "A smidge more colour"
//!
//! Six styles and no more, defined in one place below. There is deliberately
//! **no syntax highlighting**: `ADR-005` §5.5 chose "enough to fix a file" over
//! "enough to enjoy", and a highlighting engine is a grammar set on a card whose
//! size is a tracked number (`ADR-002` §5.1).
//!
//! The one place colour carries meaning rather than decoration is the title
//! bar, which turns amber the moment a read-only view is open. That is
//! `ADR-007` §3.6 delivered before the mistake rather than after it.

use crate::editor::Editor;
use crate::text::Cursor;
use crate::view::Opening;

/// The sixteen colours every terminal that has any has.
///
/// Not 256-colour and not truecolour, because this program runs on a serial
/// console as often as under an emulator, and a palette index the console
/// renders as nothing is worse than a plainer choice that always works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Black,
    Red,
    Yellow,
    Blue,
    BrightBlack,
    BrightYellow,
    BrightWhite,
}

/// How one span is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Option<Colour>,
    pub bg: Option<Colour>,
    pub bold: bool,
}

impl Style {
    /// Body text.
    pub const PLAIN: Style = Style {
        fg: None,
        bg: None,
        bold: false,
    };

    /// The title bar of an ordinary, writable file.
    pub const TITLE: Style = Style {
        fg: Some(Colour::BrightWhite),
        bg: Some(Colour::Blue),
        bold: true,
    };

    /// The title bar of a **view**. `ADR-007` §3.6: honest about being
    /// read-only, from the moment the file is on screen rather than at the
    /// moment of the write.
    pub const TITLE_VIEW: Style = Style {
        fg: Some(Colour::Black),
        bg: Some(Colour::Yellow),
        bold: true,
    };

    /// A key name in the help lines, and a prompt's label. Reverse video is
    /// what `nano` uses and is why its bottom two lines read as keys rather
    /// than as prose; this is the same effect spelled as colours, so that a
    /// terminal which renders one badly renders the other.
    pub const KEY: Style = Style {
        fg: Some(Colour::Black),
        bg: Some(Colour::BrightWhite),
        bold: false,
    };

    /// Something the operator needs to notice — a refusal, a write error.
    pub const ALERT: Style = Style {
        fg: Some(Colour::BrightWhite),
        bg: Some(Colour::Red),
        bold: true,
    };

    /// Something that went right, or a fact. Dimmer than [`Style::ALERT`] on
    /// purpose: a status line that shouts every time it says anything trains an
    /// operator to stop reading it.
    pub const NOTE: Style = Style {
        fg: Some(Colour::BrightYellow),
        bg: None,
        bold: false,
    };

    /// The line numbers down the left of a view.
    pub const GUTTER: Style = Style {
        fg: Some(Colour::BrightBlack),
        bg: None,
        bold: false,
    };
}

/// A run of characters drawn the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

impl Span {
    pub fn new(text: impl Into<String>, style: Style) -> Span {
        Span {
            text: text.into(),
            style,
        }
    }

    pub fn plain(text: impl Into<String>) -> Span {
        Span::new(text, Style::PLAIN)
    }
}

/// One rendered screen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frame {
    pub rows: Vec<Vec<Span>>,
    /// Zero-based, `(row, column)`, in screen cells.
    pub cursor: (u16, u16),
}

impl Frame {
    /// The text of one row with the styling dropped — what a test asserts on.
    ///
    /// `cfg(test)`: the running program draws spans, and only a test wants them
    /// flattened back into a string.
    #[cfg(test)]
    pub fn row(&self, index: usize) -> String {
        self.rows
            .get(index)
            .map(|spans| spans.iter().map(|s| s.text.as_str()).collect())
            .unwrap_or_default()
    }

    /// The style of the first span of a row, which is what a test asks when it
    /// wants to know the *colour* of something rather than its text.
    #[cfg(test)]
    pub fn style_of(&self, index: usize) -> Option<Style> {
        self.rows.get(index).and_then(|r| r.first()).map(|s| s.style)
    }

    /// Every row's text, joined by newlines. For assertions that mean
    /// "somewhere on screen".
    #[cfg(test)]
    pub fn text(&self) -> String {
        (0..self.rows.len())
            .map(|i| self.row(i))
            .collect::<Vec<String>>()
            .join("\n")
    }
}

/// How big the terminal is, in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

impl Size {
    /// What to assume when the terminal will not say. 80x24 is the VT100's, and
    /// it is what every terminal that has ever lied about its size lies as.
    pub const FALLBACK: Size = Size { rows: 24, cols: 80 };

    /// How many rows of the buffer are visible. Never zero, so that a two-row
    /// terminal still shows the line the cursor is on rather than dividing by
    /// nothing.
    pub fn text_rows(self) -> usize {
        usize::from(self.rows).saturating_sub(CHROME_ROWS).max(1)
    }

    /// How many columns wide the screen is. Never zero.
    pub fn text_cols(self) -> usize {
        usize::from(self.cols).max(1)
    }
}

/// Title, status, and two help lines.
const CHROME_ROWS: usize = 4;

/// One entry in the two-line key help.
pub struct HelpEntry(pub &'static str, pub &'static str);

/// The keys an editable buffer offers.
///
/// **Every one of these is `nano`'s own binding for the same act.** `^K` cuts,
/// `^U` puts back, `^W` searches, `^_` goes to a line. Nothing here is invented
/// and nothing is rebound, because a key that means something else in this
/// editor than in `nano` is worse than a key that is missing: the missing one
/// fails visibly.
pub const HELP_EDIT: &[HelpEntry] = &[
    HelpEntry("^G", "Get Help"),
    HelpEntry("^O", "Write Out"),
    HelpEntry("^W", "Where Is"),
    HelpEntry("^K", "Cut Text"),
    HelpEntry("^X", "Exit"),
    HelpEntry("^U", "Uncut Text"),
    HelpEntry("^_", "Go To Line"),
    HelpEntry("^C", "Cur Pos"),
];

/// The keys a **view** offers.
///
/// `^K` and `^U` are gone because nothing in a view can be changed, and
/// advertising a key whose only answer is "no" is worse than not offering it.
/// **`^O` stays**, and that is deliberate: it is the key a `nano` user reaches
/// for when they want to keep what they are looking at, and in a view it is the
/// key that explains why they cannot — at exactly the moment they asked. It
/// also still works, for a *writable* destination, which is how a copy of
/// `/etc/passwd` gets saved to `/state/home`.
pub const HELP_VIEW: &[HelpEntry] = &[
    HelpEntry("^G", "Get Help"),
    HelpEntry("^W", "Where Is"),
    HelpEntry("^O", "Write Out"),
    HelpEntry("^X", "Exit"),
    HelpEntry("^_", "Go To Line"),
    HelpEntry("^C", "Cur Pos"),
];

/// Render the editor.
pub fn render(editor: &Editor, size: Size) -> Frame {
    if editor.help_is_open() {
        return help_screen(editor, size);
    }

    let mut rows: Vec<Vec<Span>> = Vec::with_capacity(usize::from(size.rows));
    let view = matches!(editor.opening(), Opening::ReadOnly(_));
    rows.push(title(editor, size, view));

    let text_rows = size.text_rows().min(usize::from(size.rows).saturating_sub(1));
    let gutter = gutter_width(editor);
    let width = size.text_cols().saturating_sub(gutter);
    for offset in 0..text_rows {
        let index = editor.top() + offset;
        let mut spans: Vec<Span> = Vec::new();
        if index < editor.text().line_count() {
            if gutter > 0 {
                spans.push(Span::new(
                    format!("{:>w$} ", index + 1, w = gutter.saturating_sub(1)),
                    Style::GUTTER,
                ));
            }
            spans.push(Span::plain(window(
                editor.text().line(index),
                editor.left(),
                width,
            )));
        }
        rows.push(spans);
    }

    rows.push(status(editor, size));
    rows.extend(help_lines(if view { HELP_VIEW } else { HELP_EDIT }, size));
    rows.truncate(usize::from(size.rows).max(1));

    Frame {
        cursor: cursor_cell(editor, size, gutter, rows.len()),
        rows,
    }
}

/// The width of the line-number column, or zero if there is not one.
///
/// **Only in a view.** In edit mode the columns belong to the text: the
/// operator is typing, and `^C` answers "which line am I on" on demand. In a
/// view nobody is typing, and the numbers are what make "the third field on
/// line 4" a thing two people can say to each other over a phone.
pub fn gutter_width(editor: &Editor) -> usize {
    match editor.opening() {
        Opening::ReadOnly(_) => editor.text().line_count().to_string().len() + 1,
        Opening::Editable => 0,
    }
}

fn title(editor: &Editor, size: Size, view: bool) -> Vec<Span> {
    let style = if view { Style::TITLE_VIEW } else { Style::TITLE };
    let tag = if view {
        " VIEW — read only ".to_owned()
    } else if editor.text().is_dirty() {
        " Modified ".to_owned()
    } else {
        String::new()
    };
    let left = format!(" femto {} ", env!("CARGO_PKG_VERSION"));
    let cols = size.text_cols();

    // Left, then the name, then the tag pushed to the right — and any of them
    // may be dropped rather than allowed to wrap, because a title bar that
    // wraps eats a line of the buffer.
    let mut text = String::new();
    if left.chars().count() + tag.chars().count() < cols {
        text.push_str(&left);
    }
    let room = cols
        .saturating_sub(text.chars().count())
        .saturating_sub(tag.chars().count());
    text.push_str(&elide(editor.path(), room));
    let pad = cols
        .saturating_sub(text.chars().count())
        .saturating_sub(tag.chars().count());
    text.push_str(&" ".repeat(pad));
    text.push_str(&tag);
    vec![Span::new(clip(&text, cols), style)]
}

fn status(editor: &Editor, size: Size) -> Vec<Span> {
    let cols = size.text_cols();
    if let Some((label, answer)) = editor.prompt() {
        let label = format!("{label} ");
        let used = label.chars().count();
        return vec![
            Span::new(label, Style::KEY),
            Span::plain(clip(answer, cols.saturating_sub(used))),
        ];
    }
    match editor.message() {
        Some((text, alert)) => {
            let style = if alert { Style::ALERT } else { Style::NOTE };
            // Centred the way `nano` centres its status line, which is how an
            // operator's eye finds it without looking for it.
            let text = clip(text, cols.saturating_sub(2));
            let pad = cols.saturating_sub(text.chars().count() + 2) / 2;
            vec![
                Span::plain(" ".repeat(pad)),
                Span::new(format!(" {text} "), style),
            ]
        }
        None => vec![Span::plain(String::new())],
    }
}

fn help_lines(entries: &'static [HelpEntry], size: Size) -> Vec<Vec<Span>> {
    let cols = size.text_cols();
    let per_row = entries.len().div_ceil(2).max(1);
    let cell = (cols / per_row).max(4);
    let mut rows = Vec::with_capacity(2);
    for chunk in entries.chunks(per_row) {
        let mut spans = Vec::new();
        for HelpEntry(key, label) in chunk {
            spans.push(Span::new(*key, Style::KEY));
            let room = cell.saturating_sub(key.chars().count() + 1);
            spans.push(Span::plain(format!(" {:<room$}", clip(label, room))));
        }
        rows.push(spans);
    }
    while rows.len() < 2 {
        rows.push(vec![Span::plain(String::new())]);
    }
    rows
}

/// `^G`. A whole screen rather than a line, because a view's explanation does
/// not fit on one and truncating it would leave the useless half of a true
/// answer — which is the defect `ADR-007` §3.6 finds in a bare `EROFS`.
fn help_screen(editor: &Editor, size: Size) -> Frame {
    let cols = size.text_cols();
    let banner = format!(" femto {} — help", env!("CARGO_PKG_VERSION"));
    let mut rows: Vec<Vec<Span>> = vec![vec![Span::new(
        clip(&format!("{banner}{}", " ".repeat(cols)), cols),
        Style::TITLE,
    )]];

    if let Opening::ReadOnly(refusal) = editor.opening() {
        for line in refusal.lines(editor.path()) {
            rows.push(vec![Span::new(clip(&line, cols), Style::NOTE)]);
        }
        rows.push(vec![Span::plain(String::new())]);
    }
    for HelpEntry(key, label) in HELP_EDIT {
        rows.push(vec![
            Span::new(format!("  {key}"), Style::KEY),
            Span::plain(format!("  {label}")),
        ]);
    }
    rows.push(vec![Span::plain(String::new())]);
    for line in [
        "  Arrows, Home, End, PageUp and PageDown move. Typing inserts.",
        "  Backspace and Delete remove. There are no modes.",
        "  ^A start of line, ^E end of line, ^Y page up, ^V page down, ^D delete.",
    ] {
        rows.push(vec![Span::plain(line.to_owned())]);
    }
    rows.push(vec![Span::plain(String::new())]);
    rows.push(vec![Span::new(
        "  Press any key to go back.".to_owned(),
        Style::NOTE,
    )]);

    rows.truncate(usize::from(size.rows).max(1));
    Frame {
        cursor: (0, 0),
        rows,
    }
}

/// Where the hardware cursor goes.
fn cursor_cell(editor: &Editor, size: Size, gutter: usize, drawn: usize) -> (u16, u16) {
    if let Some((label, answer)) = editor.prompt() {
        let row = drawn.saturating_sub(3);
        let col = label.chars().count() + 1 + answer.chars().count();
        return (clamp(row), clamp(col.min(size.text_cols())));
    }
    let Cursor { line, col } = editor.cursor();
    let row = line.saturating_sub(editor.top()) + 1;
    let col = display_col(editor.text().line(line), col).saturating_sub(editor.left()) + gutter;
    (
        clamp(row.min(usize::from(size.rows).saturating_sub(1))),
        clamp(col.min(size.text_cols().saturating_sub(1))),
    )
}

fn clamp(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// The screen column a character index lands on.
///
/// **Tabs are the whole reason this function exists.** A tab is one character
/// in `crate::text` and up to eight cells on screen, so a cursor placed at the
/// character index would drift left of the glyph it is meant to be on. Width is
/// otherwise taken as one cell per character, which is wrong for CJK and for
/// combining marks — recorded rather than hidden: fixing it needs a Unicode
/// width table, which is a dependency this crate's manifest argues against for
/// the same reason it argues against the others. The consequence is cosmetic
/// (the cursor sits left of a wide glyph) and never affects what is written.
pub fn display_col(line: &str, col: usize) -> usize {
    line.chars().take(col).fold(0, |at, c| {
        if c == '\t' {
            at + TAB_WIDTH - (at % TAB_WIDTH)
        } else {
            at + 1
        }
    })
}

/// Eight, which is what a terminal does with a tab it is sent, and therefore
/// the only value that puts the cursor where the glyph is.
pub const TAB_WIDTH: usize = 8;

/// The visible slice of a line: `width` cells starting at cell `from`.
///
/// Tabs are expanded here rather than passed through, because the terminal
/// would expand them relative to *its* column zero and this line may be
/// scrolled sideways — which puts every tab stop on a horizontally scrolled
/// line in the wrong place.
fn window(line: &str, from: usize, width: usize) -> String {
    let mut out = String::new();
    let mut at = 0usize;
    for c in line.chars() {
        let next = if c == '\t' {
            at + TAB_WIDTH - (at % TAB_WIDTH)
        } else {
            at + 1
        };
        if next > from {
            if next > from + width {
                break;
            }
            if c == '\t' {
                out.push_str(&" ".repeat(next - at.max(from)));
            } else {
                out.push(c);
            }
        }
        at = next;
    }
    out
}

/// Truncate to `width` characters.
fn clip(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

/// Truncate from the *left*, with a leading ellipsis.
///
/// For paths, where the last component is the informative half —
/// `…/nushell/config.nu` says more than `/state/home/root/.config/nu…`.
fn elide(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_owned();
    }
    if width <= 1 {
        return clip(text, width);
    }
    let mut out = String::from("…");
    out.extend(text.chars().skip(count - (width - 1)));
    out
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::key::Key;
    use crate::text::Text;

    const SIZE: Size = Size { rows: 12, cols: 60 };

    fn editor(path: &str, body: &str) -> Editor {
        Editor::open(
            path.to_owned(),
            Text::from_bytes(body.as_bytes()).expect("the fixture is text"),
            crate::view::classify(path),
        )
    }

    #[test]
    fn the_title_bar_names_the_file_and_the_program() {
        let frame = render(&editor("/state/notes.txt", "hello\n"), SIZE);
        let title = frame.row(0);
        assert!(title.contains("femto"), "{title}");
        assert!(title.contains("notes.txt"), "{title}");
    }

    #[test]
    fn the_title_bar_says_modified_only_once_something_has_been_typed() {
        let mut editor = editor("/state/notes.txt", "hello\n");
        assert!(!render(&editor, SIZE).row(0).contains("Modified"));
        editor.press(Key::Char('x'));
        assert!(render(&editor, SIZE).row(0).contains("Modified"));
    }

    /// `ADR-007` §3.6: honest about being read-only, and honest before the
    /// operator has typed anything rather than at the moment of the write.
    #[test]
    fn a_view_says_so_in_the_title_bar_the_moment_it_opens() {
        let frame = render(&editor("/etc/hosts", "127.0.0.1 localhost\n"), SIZE);
        assert!(frame.row(0).contains("VIEW"), "{}", frame.row(0));
        assert_eq!(
            frame.style_of(0),
            Some(Style::TITLE_VIEW),
            "a view's title bar is a different colour from a writable file's"
        );
    }

    #[test]
    fn the_two_help_lines_are_always_the_last_two_rows() {
        let frame = render(&editor("/state/notes.txt", "hello\n"), SIZE);
        let rows = frame.rows.len();
        assert_eq!(rows, usize::from(SIZE.rows));
        assert!(frame.row(rows - 2).contains("^G"), "{}", frame.row(rows - 2));
        assert!(frame.row(rows - 1).contains("^X"), "{}", frame.row(rows - 1));
    }

    #[test]
    fn a_view_advertises_no_key_that_could_only_answer_no() {
        let frame = render(&editor("/etc/hosts", "127.0.0.1 localhost\n"), SIZE);
        let rows = frame.rows.len();
        let help = format!("{}{}", frame.row(rows - 2), frame.row(rows - 1));
        assert!(
            !help.contains("^K") && !help.contains("^U"),
            "nothing in a view can be cut or pasted: {help}"
        );
        assert!(help.contains("^X") && help.contains("^O"), "{help}");
    }

    #[test]
    fn a_view_shows_line_numbers_and_an_editable_file_does_not() {
        assert!(
            render(&editor("/etc/hosts", "a\nb\n"), SIZE)
                .row(1)
                .starts_with("1 ")
        );
        assert!(
            !render(&editor("/state/x", "a\nb\n"), SIZE)
                .row(1)
                .starts_with("1 ")
        );
    }

    #[test]
    fn the_buffer_is_drawn_below_the_title_bar() {
        let frame = render(&editor("/state/notes.txt", "first\nsecond\n"), SIZE);
        assert_eq!(frame.row(1), "first");
        assert_eq!(frame.row(2), "second");
    }

    #[test]
    fn the_cursor_lands_on_the_character_it_is_at() {
        let mut editor = editor("/state/notes.txt", "hello\nworld\n");
        editor.press(Key::Char('x'));
        // Row 1 is the first text row; column 1 is after the inserted `x`.
        assert_eq!(render(&editor, SIZE).cursor, (1, 1));
    }

    #[test]
    fn a_tab_puts_the_cursor_where_the_glyph_is_not_where_the_character_is() {
        // One character, eight cells. A cursor at the character index would sit
        // seven columns left of where the operator sees it.
        assert_eq!(display_col("\tx", 1), TAB_WIDTH);
        assert_eq!(display_col("ab\tx", 3), TAB_WIDTH);
    }

    #[test]
    fn a_prompt_replaces_the_status_line_and_takes_the_cursor() {
        let mut editor = editor("/state/notes.txt", "hello\n");
        editor.press(Key::Ctrl('W'));
        let frame = render(&editor, SIZE);
        let status = frame.row(usize::from(SIZE.rows) - 3);
        assert!(status.contains("Search"), "{status}");
        assert_eq!(frame.cursor.0, SIZE.rows - 3);
    }

    #[test]
    fn a_long_path_is_elided_from_the_left_so_the_filename_survives() {
        let elided = elide("/state/home/root/.config/nushell/config.nu", 20);
        assert!(elided.starts_with('…'), "{elided}");
        assert!(elided.ends_with("config.nu"), "{elided}");
        assert_eq!(elided.chars().count(), 20);
    }

    #[test]
    fn a_terminal_too_small_for_the_chrome_still_renders_something() {
        // Three rows is less than the chrome, so there is barely room for text
        // at all — and the renderer must not produce more rows than the screen
        // has, or the terminal scrolls and the whole frame shifts up one.
        let tiny = Size { rows: 3, cols: 20 };
        let frame = render(&editor("/state/x", "hello\n"), tiny);
        assert_eq!(frame.rows.len(), 3);
        assert!(frame.cursor.0 < tiny.rows);
    }

    #[test]
    fn a_line_wider_than_the_screen_is_windowed_rather_than_wrapped() {
        let wide = "0123456789".repeat(20);
        let frame = render(&editor("/state/x", &format!("{wide}\n")), SIZE);
        assert_eq!(frame.row(1).chars().count(), usize::from(SIZE.cols));
    }

    #[test]
    fn the_help_screen_carries_the_whole_refusal_for_a_view() {
        let mut editor = editor("/etc/hosts", "127.0.0.1 localhost\n");
        editor.press(Key::Ctrl('G'));
        let frame = render(&editor, Size { rows: 40, cols: 100 });
        let text = frame.text();
        assert!(text.contains("change with"), "{text}");
        assert!(text.contains("loopback"), "{text}");
    }

    /// A help line advertising a key that does nothing is the same defect as a
    /// catalogue naming an operation that does not exist (`ADR-006` §7).
    #[test]
    fn every_advertised_key_is_handled() {
        for HelpEntry(key, label) in HELP_EDIT.iter().chain(HELP_VIEW) {
            let letter = key.chars().nth(1).expect("every entry is `^` and a letter");
            let mut editor = editor("/state/notes.txt", "one\ntwo\n");
            assert!(
                editor.handles(Key::Ctrl(letter)),
                "the help offers {key} {label} and the editor does nothing with it"
            );
        }
    }
}
