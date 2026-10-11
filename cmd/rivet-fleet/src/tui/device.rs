//! **One device** — the screen behind `enter` on the fleet list.
//!
//! The maintainer ruled the layout on 2026-08-27: a **left nav rail**, not tabs
//! across the top. Tabs run out of horizontal room in a terminal as sections are
//! added, and a rail carries a per-section badge without crowding.
//!
//! He also ruled what he wants to *see*: switches to turn things on or off and
//! fields to enter hostnames and the like into (.rulings/ R-327), and
//! explicitly not TOML and not TOML comments. So every pane below renders
//! **values**, and the document is never shown.
//!
//! # The device is held by identifier
//!
//! Not by index into a list and not by label. `ADR-025` §4.1 makes the
//! identifier authoritative and the label mutable, and both of the other two
//! can change while this screen is open — a probe answering reorders nothing,
//! but hiding a device or renaming one would move an index.
//!
//! # What is read-only here, and why that is said out loud
//!
//! **Everything, for now.** The panes draw what the register holds; nothing on
//! this screen edits yet, and each pane says so rather than presenting a control
//! that does nothing. A switch that does not switch is the same defect as a
//! footer advertising a key that does nothing — which is what this screen was
//! written to fix.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::register::{Device, Register};
use crate::tui::edit::Draft;
use crate::tui::log::{Entry, Fetched};
use crate::tui::reach::Reach;
use crate::tui::theme::{Glyphs, Palette};

/// The sections in the rail, in the order they appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Status,
    /// **What the machine is doing right now** — `crate::tui::top`, added
    /// 2026-08-29. Every other pane draws the *register's* record of a device;
    /// this one draws the device. It is second in the rail because `ADR-035`
    /// makes it the thing a person opens the tool for once there is no shell to
    /// open instead.
    Monitor,
    System,
    Network,
    Services,
    Repositories,
    Journal,
}

impl Pane {
    /// Every pane, in rail order.
    ///
    /// **Ordered from what a machine IS to what it SERVES**: what it is doing
    /// now, then its identity, then how it is reached, then what runs on it,
    /// then who may reach it, then what they get. A person looking for
    /// something scans down that.
    pub const ALL: &'static [Pane] = &[
        Pane::Status,
        Pane::Monitor,
        Pane::System,
        Pane::Network,
        Pane::Services,
        Pane::Repositories,
        Pane::Journal,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Monitor => "Monitor",
            Self::System => "System",
            Self::Network => "Network",
            Self::Services => "Services",
            Self::Repositories => "Repositories",
            Self::Journal => "Journal",
        }
    }
}

/// What the device screen is showing.
pub struct DeviceScreen {
    /// **The identifier**, never an index — see the module header.
    pub id: String,
    pub pane: Pane,
    /// Which row within the pane, for the panes that have rows.
    pub row: usize,
    /// What the journal pane is showing, once something has answered.
    pub log: Option<Fetched>,
    /// What a person has typed into the journal's filter.
    pub filter: String,
    /// Whether the filter is taking keystrokes.
    pub filtering: bool,
    /// Whether the journal re-asks on a timer.
    pub following: bool,
    /// Whether the journal shows only `warn` and `error`.
    pub trouble_only: bool,
    /// What the monitor pane is showing — **not what it has fetched**, which is
    /// `crate::telemetry::Poller`'s and lives on the application rather than on
    /// the screen so that a reader thread is not created and destroyed as a
    /// person walks the rail.
    pub monitor: crate::tui::top::Monitor,
}

impl DeviceScreen {
    #[must_use]
    pub fn opening(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            pane: Pane::Status,
            row: 0,
            log: None,
            filter: String::new(),
            filtering: false,
            following: false,
            trouble_only: false,
            monitor: crate::tui::top::Monitor::new(),
        }
    }

    /// The journal lines this screen would show, after its filters.
    ///
    /// **Filtered here rather than at draw time**, so the same list can be
    /// asserted without a terminal — and so a filter is applied once per fetch
    /// rather than once per keystroke.
    #[must_use]
    pub fn visible_log(&self) -> Vec<&Entry> {
        let Some(Fetched::Entries(entries)) = &self.log else {
            return Vec::new();
        };
        entries
            .iter()
            .filter(|e| !self.trouble_only || e.is_trouble())
            .filter(|e| e.matches(&self.filter))
            .collect()
    }

    /// The device this screen is about, if the register still has it.
    ///
    /// **`None` is possible and is not a bug**: a register can be reloaded
    /// while this screen is open. The caller returns to the fleet rather than
    /// showing a screen about nothing.
    #[must_use]
    pub fn device<'a>(&self, register: &'a Register) -> Option<&'a Device> {
        register.devices.get(&self.id)
    }

    /// Move down or up the rail, wrapping.
    pub fn step_pane(&mut self, by: isize) {
        let count = isize::try_from(Pane::ALL.len()).unwrap_or(1);
        let here = isize::try_from(
            Pane::ALL
                .iter()
                .position(|p| *p == self.pane)
                .unwrap_or_default(),
        )
        .unwrap_or(0);
        let next = (here + by).rem_euclid(count);
        self.pane = Pane::ALL[usize::try_from(next).unwrap_or(0)];
        // A new pane starts at its top; carrying a row index across panes of
        // different lengths would put the cursor somewhere nobody chose.
        self.row = 0;
    }
}

/// Draw the whole screen.
///
/// **Eight arguments, and a struct to hold them would exist only to satisfy a
/// lint.** Every one is a different thing the screen depends on and none of
/// them belong together: the register's record, the person's unsaved edits,
/// what the network said, and how this terminal renders. Grouping them would
/// name a thing that is not a thing.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    screen: &DeviceScreen,
    device: &Device,
    draft: &Draft,
    reach: Reach,
    // Why the probe said what it said, when it had something to say —
    // `crate::tui::reach::Answer::detail`.
    reach_detail: Option<&str>,
    fleet: &crate::register::Fleet,
    glyphs: Glyphs,
    palette: Palette,
    // What this machine has said about itself, and when. `None` until the
    // monitor pane has been opened for it — see `crate::tui::top::draw`.
    monitor: Option<&crate::telemetry::Poller>,
    // Why there is none, when there is none — `crate::operator`'s sentence.
    no_reader: Option<&str>,
    now: std::time::Instant,
) {
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);
    let columns =
        Layout::horizontal([Constraint::Length(16), Constraint::Min(20)]).split(rows[0]);

    frame.render_widget(rail(screen, glyphs, palette), columns[0]);
    frame.render_widget(
        Paragraph::new(body(
            screen,
            device,
            draft,
            reach,
            reach_detail,
            fleet,
            glyphs,
            palette,
            columns[1].width,
            // Less the border the block draws on both sides.
            columns[1].height.saturating_sub(2),
            monitor,
            no_reader,
            now,
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(header(device, draft, reach)),
        ),
        columns[1],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            match screen.pane {
                Pane::Services => " [tab] section  [space] toggle  [s]ave  [esc] back  [q]uit",
                // **Every key this pane answers is named here**, which is the
                // other half of `crate::tui::top::Monitor::kernel_threads`'
                // promise: a filter that is on by default has to advertise the
                // key that turns it off.
                Pane::Monitor => {
                    " [tab] section  [K]ernel threads  [d]elay  [o]rder  [p]ause  [r]efresh  \
                     [↑↓] scroll  [esc] back"
                }
                Pane::Journal if screen.filtering => " typing a filter — [enter] done  [esc] clear",
                Pane::Journal => {
                    " [tab] section  [g]et  [f]ollow  [/]filter  [w]arn+error  [esc] back  [q]uit"
                }
                _ => " [tab]/[↑↓] section  [esc] back to the fleet  [q]uit",
            },
            palette.faint(),
        ))),
        rows[1],
    );
}

/// The title bar: which machine, where, whether it answered, and whether
/// anything is waiting to be saved.
fn header(device: &Device, draft: &Draft, reach: Reach) -> String {
    let mut title = format!(" {} ", device.name);
    if let Some(address) = device
        .interfaces
        .values()
        .find_map(|i| i.address.as_ref())
        .map(|a| a.split('/').next().unwrap_or(a).to_string())
    {
        title.push_str(&format!("· {address} "));
    }
    title.push_str(&format!("· {} ", reach.word()));
    if !draft.is_empty() {
        // **Said in the title, because it is true of the whole screen.** A
        // person who drafts a change, walks the rail and comes back should not
        // have to find the pane again to remember there is something unsaved.
        title.push_str(&format!("· {} unsaved ", draft.changes(device).len()));
    }
    title
}

/// Draw the confirmation over whatever is behind it.
///
/// **Centred and small**, so the screen it interrupts is still visible around
/// it — a person answering *apply to this machine?* should be able to see which
/// machine without dismissing the question.
pub fn draw_confirm(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    confirm: &crate::tui::edit::Confirm,
    palette: Palette,
) {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Save {} change(s) to {}?", confirm.changes.len(), confirm.device),
            palette.plain(),
        )),
        Line::from(""),
    ];
    for change in &confirm.changes {
        lines.push(Line::from(Span::styled(
            format!("    {}", change.line()),
            palette.pending(),
        )));
    }
    lines.push(Line::from(""));
    // **Nothing is focused**, ruled 2026-08-27 — so the keys are spelled out
    // and `enter` is deliberately not one of them.
    lines.push(Line::from(Span::styled(
        "    [A]pply now      [L]ater      [Esc] cancel",
        palette.plain(),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "    Later writes the register only; the machine finds out at the next apply.",
        palette.faint(),
    )));

    let height = u16::try_from(lines.len() + 2).unwrap_or(12).min(area.height);
    let width = 74.min(area.width);
    let box_area = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    // Cleared first, or the screen behind shows through the gaps.
    frame.render_widget(ratatui::widgets::Clear, box_area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Confirm "),
        ),
        box_area,
    );
}

/// The nav rail.
fn rail(screen: &DeviceScreen, glyphs: Glyphs, palette: Palette) -> Paragraph<'static> {
    let lines: Vec<Line> = Pane::ALL
        .iter()
        .map(|pane| {
            let here = *pane == screen.pane;
            let text = format!(
                "{} {:<13}",
                if here { glyphs.cursor } else { ' ' },
                pane.label()
            );
            let line = Line::from(Span::styled(text, palette.plain()));
            if here { line.style(palette.selected()) } else { line }
        })
        .collect();
    Paragraph::new(lines).block(Block::default().borders(Borders::ALL))
}

/// The pane's contents.
///
/// **Ten arguments, and `draw`'s note applies unchanged**: every one is a
/// different thing a pane depends on and grouping them would name a thing that
/// is not a thing. The last four arrived with the monitor, which is the first
/// pane that draws the *machine* rather than the register's record of it.
#[allow(clippy::too_many_arguments)]
fn body(
    screen: &DeviceScreen,
    device: &Device,
    draft: &Draft,
    reach: Reach,
    reach_detail: Option<&str>,
    fleet: &crate::register::Fleet,
    glyphs: Glyphs,
    palette: Palette,
    // The pane own width, so a message longer than it can be wrapped rather
    // than clipped.
    width: u16,
    // And its height, so the monitor's process table takes what is left rather
    // than pushing the sections above it off the screen.
    height: u16,
    monitor: Option<&crate::telemetry::Poller>,
    no_reader: Option<&str>,
    now: std::time::Instant,
) -> Vec<Line<'static>> {
    match screen.pane {
        Pane::Status => status(device, reach, reach_detail, width, palette),
        Pane::Monitor => crate::tui::top::draw(
            &screen.monitor,
            monitor,
            no_reader,
            now,
            glyphs,
            palette,
            width,
            height,
        ),
        Pane::System => system(device, fleet, palette),
        Pane::Network => network(device, palette),
        Pane::Services => services(screen, device, draft, glyphs, palette),
        Pane::Repositories => repositories(device, palette),
        Pane::Journal => journal(screen, glyphs, palette, width),
    }
}

/// A `name  value` line, which is what most of this screen is.
/// The width of the label column.
///
/// **Fourteen, and a label that fills it still gets a space.** It was
/// `{name:<14}` with nothing after it until 2026-08-29, so a fourteen-character
/// label ran straight into its value — `Trusts signingnone`, read off a real
/// terminal rather than reasoned about. `{:<14}` pads a SHORT name and does
/// nothing for one that is already long enough, which is exactly the case a
/// fixed format string does not cover.
const LABEL: usize = 14;

fn field(name: &str, value: &str, palette: Palette) -> Line<'static> {
    Line::from(vec![
        // The explicit space is the fix: padding to the column and then always
        // separating, rather than trusting the padding to do both jobs.
        Span::styled(format!("  {name:<LABEL$} "), palette.faint()),
        Span::styled(value.to_owned(), palette.plain()),
    ])
}

/// A sentence about something this screen does not know or cannot do yet.
///
/// **Every pane that is incomplete says so in one of these.** `AGENTS.md` §3:
/// a stub that refuses is worth more than a control that looks like it works.
/// Break `text` into lines that fit `width`, at spaces where there is one.
///
/// **Two columns of margin**, matching every other line this module emits: the
/// leading two spaces are part of the pane's shape, not part of the message.
///
/// A word longer than the whole width - a URL, a digest, a path - is cut rather
/// than allowed to run past the frame, because a pane that overflows corrupts
/// the box around it and not just the line.
pub(super) fn wrapped(text: &str, width: u16) -> Vec<String> {
    let room = usize::from(width).saturating_sub(4).max(8);
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if word.chars().count() > room {
            if !line.is_empty() {
                out.push(format!("  {line}"));
                line = String::new();
            }
            let mut rest = word.chars().peekable();
            while rest.peek().is_some() {
                let chunk: String = rest.by_ref().take(room).collect();
                out.push(format!("  {chunk}"));
            }
            continue;
        }
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > room {
            out.push(format!("  {line}"));
            line = String::new();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(format!("  {line}"));
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn note(text: &str, palette: Palette) -> Line<'static> {
    Line::from(Span::styled(format!("  {text}"), palette.faint()))
}

/// The status pane.
///
/// **What it says about reachability is whatever `crate::tui::reach` actually
/// probed, in that module's words.** This pane once described a probe it no
/// longer had — a sentence that survived its own subject, which is `AGENTS.md`
/// §7's *a claim that was true and rotted* — so the words follow the probe now
/// rather than being written here.
fn status(
    device: &Device,
    reach: Reach,
    detail: Option<&str>,
    width: u16,
    palette: Palette,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        field("Reachable", reach.word(), palette),
        field("Identifier", &device.id, palette),
        field("Role", device.role.as_deref().unwrap_or("—"), palette),
        field("Location", device.location.as_deref().unwrap_or("—"), palette),
        Line::from(""),
    ];
    // **The reason, when there is one.** A pane that says `STRANGER` and will
    // not say why has told a person something is wrong and left them nowhere to
    // start — `AGENTS.md` §5, applied to a word this screen chose itself.
    // **Wrapped, not clipped**, which is `PLAN.md` §55's defect in this pane
    // rather than in the monitor's: the refusals these carry put the half that
    // says what to do at the END, so a message cut at the frame loses exactly
    // the useful part. Found on 2026-08-29 by running the interface in a pty
    // and reading the screen — the no-identity refusal stopped at "can reach
    // no m".
    if let Some(said) = detail {
        for source in said.lines() {
            // Less `note`'s own two columns — see the explanation below.
            for line in wrapped(source, width.saturating_sub(2)) {
                lines.push(note(&line, palette));
            }
        }
        lines.push(Line::from(""));
    }
    // **Wrapped to the pane rather than hand-broken.** Lines broken by hand fit
    // one width and overflow every narrower one, which is the same clipping
    // defect wearing a different hat.
    for line in wrapped(
        "Reachable means the control plane answered at that address AND presented the key \
         this register holds (ADR-025 §2). It does NOT mean the machine accepts THIS \
         operator, or that Rivet is healthy — the monitor pane is where the machine answers \
         for itself.",
        // **Less `note`'s own two columns.** `wrapped` reserves a two-column
        // margin on each side and `note` then indents by two more, so passing
        // the raw width produces lines exactly one or two columns too wide —
        // which reads as a clip at the right border. Seen in a pty at 120×40
        // before it was believed.
        width.saturating_sub(2),
    ) {
        lines.push(note(&line, palette));
    }
    lines
}

fn system(
    device: &Device,
    fleet: &crate::register::Fleet,
    palette: Palette,
) -> Vec<Line<'static>> {
    // **Composed, not read off the record.** `Device::control_plane_keys` is
    // only this device's half; `crate::compose::control_plane_keys` layers the
    // fleet's under it and is what actually reaches the machine's `peers` file.
    // Showing the record's half alone was reporting a smaller number than the
    // board holds.
    let peers = crate::compose::control_plane_keys(device, fleet);
    vec![
        Line::from(""),
        // The hostname a machine gets is its label — `crate::compose::hostname`
        // is the authority and this shows what it will produce, not a second
        // guess at it.
        field("Hostname", &device.name, palette),
        field(
            "Listens on",
            device
                .listen
                .as_deref()
                .unwrap_or(rivet_config::control_plane::DEFAULT_LISTEN),
            palette,
        ),
        field(
            "Its own key",
            device.pin.as_deref().unwrap_or("— (set pin, or add identity)"),
            palette,
        ),
        // **Renamed 2026-08-29.** This said "Trusted keys" and showed
        // `Device::trusts`, which is `ADR-028` §5.5's SIGNING key ids — so a
        // board holding two control-plane peers read as `none` on this screen.
        // Two different trusts, and the label named neither.
        field(
            "Trusts signing",
            &if device.trusts.is_empty() {
                "none".to_string()
            } else {
                device.trusts.join(", ")
            },
            palette,
        ),
        field(
            "Accepts keys",
            &if peers.is_empty() {
                "none".to_string()
            } else {
                format!("{} key(s)", peers.len())
            },
            palette,
        ),
        Line::from(""),
        note("Read-only for now. Editing is the next piece.", palette),
        note(
            "\"Accepts keys\" is what compose writes to the machine's peers",
            palette,
        ),
        note("file — this device's keys and the fleet's, layered.", palette),
    ]
}

fn network(device: &Device, palette: Palette) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from("")];
    if device.interfaces.is_empty() {
        lines.push(note(
            "No interface. A card burned from this record boots and cannot be reached.",
            palette,
        ));
        return lines;
    }
    for (name, interface) in &device.interfaces {
        lines.push(Line::from(Span::styled(
            format!("  {name}"),
            palette.plain(),
        )));
        lines.push(field("  method", &interface.method, palette));
        if let Some(address) = &interface.address {
            lines.push(field("  address", address, palette));
        }
        if let Some(gateway) = &interface.gateway {
            lines.push(field("  gateway", gateway, palette));
        }
        lines.push(Line::from(""));
    }
    lines
}

/// The service names this pane offers a switch for.
///
/// **The device's own, and nothing invented.** A pane that listed every service
/// Rivet has would offer switches for things this record says nothing about,
/// and turning one "off" would write an answer where silence meant *take the
/// fleet's*.
#[must_use]
pub fn service_names(device: &Device) -> Vec<String> {
    device.services.keys().cloned().collect()
}

fn services(
    screen: &DeviceScreen,
    device: &Device,
    draft: &Draft,
    glyphs: Glyphs,
    palette: Palette,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from("")];
    let names = service_names(device);
    if names.is_empty() {
        lines.push(note(
            "This device sets no service of its own; it takes the fleet's answers.",
            palette,
        ));
        return lines;
    }
    for (at, name) in names.iter().enumerate() {
        // **The draft, not the record** — a flipped switch looks flipped at
        // once, which is the whole of the flow he asked for (.rulings/ R-329).
        let on = draft.service(device, name);
        let (glyph, style) = if on {
            (glyphs.on, palette.up())
        } else {
            (glyphs.off, palette.faint())
        };
        let mut spans = vec![
            Span::styled(
                if at == screen.row {
                    format!("{} ", glyphs.cursor)
                } else {
                    "  ".to_string()
                },
                palette.plain(),
            ),
            Span::styled(format!("{glyph} "), style),
            Span::styled(format!("{name:<18}"), palette.plain()),
            // The word, never the glyph alone — `crate::tui::theme`'s rule.
            Span::styled(format!("{:<5}", if on { "on" } else { "off" }), style),
        ];
        if draft.service_changed(name) {
            spans.push(Span::styled(
                format!("{} unsaved", glyphs.pending),
                palette.pending(),
            ));
        }
        let line = Line::from(spans);
        lines.push(if at == screen.row {
            line.style(palette.selected())
        } else {
            line
        });
    }
    lines
}

fn repositories(device: &Device, palette: Palette) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from("")];
    if device.repos.is_empty() {
        lines.push(note("No repository. `add repo` declares one.", palette));
        return lines;
    }
    for (name, id) in &device.repos {
        lines.push(Line::from(Span::styled(
            format!("  {name}"),
            palette.plain(),
        )));
        lines.push(field("  id", id, palette));
        lines.push(Line::from(""));
    }
    lines
}

fn journal(
    screen: &DeviceScreen,
    glyphs: Glyphs,
    palette: Palette,
    width: u16,
) -> Vec<Line<'static>> {
    // The controls, always, so a person knows what the pane can do before it
    // has anything to show.
    let mut lines = vec![Line::from(vec![
        Span::styled("  filter ", palette.faint()),
        Span::styled(
            format!(
                "[{}]{} ",
                if screen.filter.is_empty() { " ".repeat(12) } else { format!("{:<12}", screen.filter) },
                if screen.filtering { "*" } else { " " }
            ),
            if screen.filtering { palette.pending() } else { palette.plain() },
        ),
        Span::styled(
            format!("{} {}   ", glyphs.warn, if screen.trouble_only { "warn+error only" } else { "all severities" }),
            if screen.trouble_only { palette.warn() } else { palette.faint() },
        ),
        Span::styled(
            format!("{} {}", glyphs.restart, if screen.following { "following" } else { "not following" }),
            if screen.following { palette.up() } else { palette.faint() },
        ),
    ])];
    lines.push(Line::from(""));

    match &screen.log {
        None => {
            lines.push(note("Nothing asked for yet — press `g` to fetch.", palette));
            return lines;
        }
        // **A failure is shown.** A pane that quietly showed nothing would look
        // the same as a machine with nothing to say.
        Some(Fetched::Failed(why)) => {
            // **Wrapped, because the useful half is at the end.** A refusal
            // this pane clipped at the frame read `... failed on
            // root@10.0.0.2 (exit st` and stopped - the exit status, the
            // reason and what to do about it were all past the edge. Found by
            // running the interface, 2026-08-27; `PLAN.md` 55.
            for line in wrapped(&format!("the machine did not answer: {why}"), width) {
                lines.push(Line::from(Span::styled(line, palette.error())));
            }
            return lines;
        }
        Some(Fetched::Entries(_)) => {}
    }

    let shown = screen.visible_log();
    if shown.is_empty() {
        lines.push(note(
            "No line matches. `/` edits the filter, `w` toggles warn+error only.",
            palette,
        ));
        return lines;
    }
    for entry in shown {
        let style = match entry.severity.as_str() {
            "error" => palette.error(),
            "warn" => palette.warn(),
            _ => palette.plain(),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<6}", entry.severity), style),
            Span::styled(format!("{:<14}", entry.service), palette.faint()),
            Span::styled(entry.message.clone(), palette.plain()),
        ]));
    }
    lines
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn device() -> Device {
        Device {
            id: "4f2c9a01bd7e3865".to_string(),
            name: "board".to_string(),
            ..Device::default()
        }
    }

    /// The rail wraps in both directions, so a person pressing a key always
    /// sees something move.
    #[test]
    fn the_rail_wraps_at_both_ends() {
        let mut screen = DeviceScreen::opening("an-id");
        assert_eq!(screen.pane, Pane::Status);
        screen.step_pane(-1);
        assert_eq!(screen.pane, *Pane::ALL.last().expect("panes"));
        screen.step_pane(1);
        assert_eq!(screen.pane, Pane::Status);
    }

    /// Every pane has a label, because the rail is the only place a person
    /// learns what the sections are.
    #[test]
    fn every_pane_says_what_it_is() {
        for pane in Pane::ALL {
            assert!(!pane.label().is_empty(), "{pane:?}");
        }
        // And no two share a label, which would make the rail ambiguous.
        let mut labels: Vec<&str> = Pane::ALL.iter().map(|p| p.label()).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "two panes share a label");
    }

    /// **Moving to a new pane starts at its top.** Carrying a row index across
    /// panes of different lengths puts the cursor somewhere nobody chose.
    #[test]
    fn changing_pane_resets_the_row() {
        let mut screen = DeviceScreen::opening("an-id");
        screen.row = 4;
        screen.step_pane(1);
        assert_eq!(screen.row, 0);
    }

    /// **The screen is held by identifier**, so a rename or a reload does not
    /// point it at a different machine.
    #[test]
    fn the_screen_follows_the_identifier_and_not_the_label() {
        let mut register = Register::empty(std::path::PathBuf::from("/nowhere"));
        register.devices.insert("4f2c9a01bd7e3865".to_string(), device());
        let screen = DeviceScreen::opening("4f2c9a01bd7e3865");

        assert_eq!(screen.device(&register).map(|d| d.name.as_str()), Some("board"));

        // Renamed: same screen, same machine.
        register
            .devices
            .get_mut("4f2c9a01bd7e3865")
            .expect("the device")
            .name = "kitchen".to_string();
        assert_eq!(screen.device(&register).map(|d| d.name.as_str()), Some("kitchen"));
    }

    /// A device that is gone leaves the screen with nothing to show, and that
    /// is an answer rather than a panic — a register can be reloaded while this
    /// screen is open.
    #[test]
    fn a_device_that_vanished_is_none_rather_than_a_crash() {
        let register = Register::empty(std::path::PathBuf::from("/nowhere"));
        assert!(DeviceScreen::opening("gone").device(&register).is_none());
    }

    /// **Every pane renders something**, including for a device that declares
    /// nothing — an empty pane with no sentence in it reads as broken.
    #[test]
    fn no_pane_is_ever_blank_even_for_a_device_that_declares_nothing() {
        let (glyphs, palette) = (Glyphs::ascii(), Palette::from_env());
        let device = device();
        for pane in Pane::ALL {
            let screen = DeviceScreen {
                id: device.id.clone(),
                pane: *pane,
                ..DeviceScreen::opening(&device.id)
            };
            let draft = Draft::for_device(&device.id);
            let lines = body(
                &screen,
                &device,
                &draft,
                Reach::Unknown,
                None,
                &crate::register::Fleet::default(),
                glyphs,
                palette,
                100,
                40,
                None,
                None,
                std::time::Instant::now(),
            );
            let text: String = lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .map(|s| s.content.as_ref())
                .collect();
            assert!(
                text.trim().len() > 10,
                "{} renders nothing a person could read: {text:?}",
                pane.label()
            );
        }
    }

    /// **A drafted switch is marked on screen and in the title**, so a person
    /// who walked away from the pane still knows something is waiting.
    #[test]
    fn a_drafted_change_is_visible_without_looking_for_it() {
        let mut device = device();
        device.services.insert("rivet-mdnsd".to_string(), true);
        let mut draft = Draft::for_device(&device.id);
        draft.toggle_service(&device, "rivet-mdnsd");

        let screen = DeviceScreen {
            id: device.id.clone(),
            pane: Pane::Services,
            ..DeviceScreen::opening(&device.id)
        };
        let text: String = services(&screen, &device, &draft, Glyphs::ascii(), Palette::from_env())
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("unsaved"), "{text:?}");
        // The switch shows the DRAFT, not the record.
        assert!(text.contains("off"), "{text:?}");

        assert!(header(&device, &draft, Reach::Up).contains("1 unsaved"));
    }

    /// **A pane that is not finished says so.** `AGENTS.md` §3: a stub that
    /// refuses is worth more than a control that looks like it works — which is
    /// the defect this whole screen was written to fix.
    #[test]
    fn a_pane_that_cannot_do_its_job_yet_admits_it() {
        let (glyphs, palette) = (Glyphs::ascii(), Palette::from_env());
        let device = device();
        for (pane, wanted) in [
            // **`"not wired to this screen yet"` until 2026-08-29**, when the
            // monitor pane wired it — so the sentence admitting a gap became a
            // sentence describing one that had been closed. What Status still
            // owes is the LIMIT of what its own word means, which is a stronger
            // admission than the one it replaced: a green dot that does not
            // prove the machine accepts this operator.
            (Pane::Status, "does NOT mean"),
            (Pane::System, "Read-only for now"),
            (Pane::Journal, "press `g` to fetch"),
        ] {
            let screen = DeviceScreen {
                id: device.id.clone(),
                pane,
                ..DeviceScreen::opening(&device.id)
            };
            let draft = Draft::for_device(&device.id);
            let text: String = body(
                &screen,
                &device,
                &draft,
                Reach::Unknown,
                None,
                &crate::register::Fleet::default(),
                glyphs,
                palette,
                100,
                40,
                None,
                None,
                std::time::Instant::now(),
            )
                .iter()
                .flat_map(|l| l.spans.iter())
                .map(|s| s.content.as_ref())
                .collect();
            assert!(
                text.contains(wanted),
                "{} does not admit what it cannot do: {text:?}",
                pane.label()
            );
        }
    }

    /// The title says which machine and whether it answered — the two things a
    /// person needs to know they are looking at the right screen.
    #[test]
    fn the_header_names_the_machine_and_its_reachability() {
        let title = header(&device(), &Draft::for_device("an-id"), Reach::Up);
        assert!(title.contains("board"), "{title}");
        assert!(title.contains("up"), "{title}");
    }

    /// **A probe that found trouble says what it found, on the screen.**
    ///
    /// `crate::tui::reach` distinguishes *answered with the wrong key* from
    /// *did not answer*; that distinction is worth nothing if the pane shows a
    /// word and swallows the reason. A red state with no reason tells a person
    /// something is wrong and leaves them nowhere to start.
    #[test]
    fn a_probes_reason_reaches_the_screen_rather_than_being_swallowed() {
        let palette = Palette::from_env();
        let said = "10.0.0.2:7777 answered and this end would not trust it: not pinned";
        let lines = status(&device(), Reach::Stranger, Some(said), 100, palette);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("STRANGER"), "the word is missing: {text:?}");
        assert!(text.contains("not pinned"), "the reason is missing: {text:?}");
    }

    /// **And it is wrapped rather than clipped**, which is `PLAN.md` §55's
    /// defect arriving in a second pane.
    ///
    /// Found on 2026-08-29 by running the interface in a pty and reading the
    /// screen: the no-identity refusal stopped at *"can reach no m"*, and the
    /// half of it that says what to do is at the END. No type-check would have
    /// caught it, and neither did the test above — which is why this one asserts
    /// the tail rather than the head.
    #[test]
    fn a_long_reason_on_the_status_pane_is_wrapped_rather_than_clipped() {
        let palette = Palette::from_env();
        let said = crate::operator::Operator::default().refusal();
        let lines = status(&device(), Reach::Unasked, Some(&said), 60, palette);
        for line in &lines {
            let width: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            // **Strictly inside**, not merely equal to it: a line the exact
            // width of the pane reads as a clip against the border, which is
            // what a 120-column pty showed on 2026-08-29.
            assert!(width < 60, "a line reaches the frame: {width} >= 60");
        }
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        // **The last words of the refusal**, which are the part a clip takes.
        // Asserted as the final token rather than as a phrase, because wrapping
        // is allowed to put a line break inside `--identity <dir>` and a test
        // that forbade that would be pinning the wrap width instead of the
        // property.
        assert!(
            text.trim_end().ends_with("answers for itself."),
            "the explanation was cut: {text:?}"
        );
        assert!(
            text.contains("Then start with"),
            "the half that says what to do was lost: {text:?}"
        );
    }

    /// **The System pane names the two trusts separately**, because they are
    /// two different things and it used to name only one of them — as
    /// *"Trusted keys"*, showing `Device::trusts` (signing key ids, `ADR-028`
    /// §5.5). A board holding two control-plane peers read as `none`, which is
    /// how this was found: on a real machine, on 2026-08-29.
    #[test]
    fn the_two_kinds_of_trust_are_named_apart_and_counted_from_the_composed_set() {
        let mut device = device();
        device.control_plane_keys.push("aa".repeat(32));
        let mut fleet = crate::register::Fleet::default();
        fleet.control_plane_keys.push("bb".repeat(32));

        let lines = system(&device, &fleet, palette_for_test());
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("Trusts signing"), "{text:?}");
        // **A space between the label and the value.** `{:<14}` pads a short
        // name and does nothing for one already that long, so `Trusts signing`
        // ran into `none` on a real terminal. Asserted as the rendered line,
        // because that is where it was wrong.
        assert!(!text.contains("signingnone"), "the label ran into its value: {text:?}");
        // **Two**, which is the device's plus the fleet's — the number the
        // machine's `peers` file will actually hold. Reading the record's own
        // field alone would say one.
        assert!(text.contains("2 key(s)"), "the fleet's key was not counted: {text:?}");
    }

    fn palette_for_test() -> Palette {
        Palette::from_env()
    }

    /// **Every label this module writes fits its column**, so none of them can
    /// repeat `Trusts signing`'s collision — which was invisible to every test
    /// here until the interface was run in a pty and the screen was read.
    #[test]
    fn no_label_runs_into_the_value_beside_it() {
        let palette = Palette::from_env();
        let mut device = device();
        device.pin = Some("ab".repeat(32));
        device.listen = Some("0.0.0.0:7777".to_owned());
        let fleet = crate::register::Fleet::default();
        let panes = [
            status(&device, Reach::Up, None, 100, palette),
            system(&device, &fleet, palette),
        ];
        for lines in panes {
            for line in lines {
                // A field line is exactly two spans: the padded label, then the
                // value. Anything else is a note and has no column to keep.
                if line.spans.len() != 2 {
                    continue;
                }
                let label = line.spans[0].content.as_ref();
                assert!(
                    label.ends_with(' '),
                    "{label:?} touches the value beside it"
                );
                assert!(
                    label.chars().count() >= LABEL + 3,
                    "{label:?} is narrower than the column plus its margin"
                );
            }
        }
    }

    /// The defect: a transport's refusal is long, and the half that says what to
    /// do is at the end of it. A pane that clipped stopped at `(exit st`.
    /// `PLAN.md` 55.
    #[test]
    fn a_long_refusal_is_wrapped_rather_than_clipped() {
        let why = "`rivet \"get\" \"log\"` failed on 10.0.0.2:7777 (exit status: 255): \
                   the peer is not pinned. Record its key with `add peer` and try again.";
        let lines = wrapped(why, 104);
        assert!(lines.len() > 1, "{lines:?}");
        for line in &lines {
            assert!(line.chars().count() <= 104, "{line:?} is wider than the pane");
            assert!(line.starts_with("  "), "{line:?} lost the margin");
        }
        // Nothing is lost: every word survives somewhere.
        let joined = lines.join(" ");
        for word in why.split_whitespace() {
            assert!(joined.contains(word), "{word:?} did not survive: {lines:?}");
        }
    }

    /// A digest, a URL or a path can be longer than the pane on its own. It is
    /// cut, because a line that ran past the frame would corrupt the box and
    /// not just itself.
    #[test]
    fn a_word_wider_than_the_pane_is_cut_rather_than_allowed_to_overflow() {
        let long = "z".repeat(300);
        for line in wrapped(&long, 40) {
            assert!(line.chars().count() <= 40, "{line:?}");
        }
    }

    /// A pane too narrow to hold anything still answers with lines, and the
    /// floor keeps them usable rather than one character wide.
    #[test]
    fn a_very_narrow_pane_still_produces_lines() {
        for width in [0_u16, 1, 4, 5] {
            let lines = wrapped("the machine did not answer", width);
            assert!(!lines.is_empty(), "width {width}");
        }
    }
}
