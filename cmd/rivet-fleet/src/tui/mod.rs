//! **The full-screen interface** — `ADR-034` §4, and the shape §4.4 ruled.
//!
//! > Ruled 2026-08-26: full-screen and menu-driven, task-oriented, aimed at a
//! > human who wants to accomplish something rather than one who wants to watch
//! > numbers. A dashboard is a different product.
//!
//! `raspi-config`, not a dashboard. Arrow keys and Enter; no mouse, no panes,
//! no live figures.
//!
//! # Every keystroke resolves to a named operation
//!
//! §4.1 is the rule and it is not "build both":
//!
//! > **Every keystroke in the human interface resolves to a named operation,
//! > and there is no operation the human interface can perform that the
//! > command-line surface cannot.**
//!
//! The menu below is [`crate::operation::OPERATIONS`], walked. There is no list
//! of screens and no `match` on a menu index — an operation appears here
//! because it is in the table, and an operation this file could offer that the
//! command line cannot is unwritable, because the only thing it can offer is a
//! row of that table.
//!
//! # It shows its work, and that is the feature rather than a debug affordance
//!
//! §4.2, in its own words: displaying the command teaches the command line to a
//! human who did not set out to learn it, it is an audit surface, and it is
//! where an agent operator and a human operator are demonstrably doing the same
//! thing rather than two similar things.
//!
//! So the pane at the foot of the screen always holds the **exact command line**
//! the selected operation would run — generated from the same table and the same
//! flag names `clap` parses, so it is the command and not a description of one.
//!
//! # What this build does and does not do
//!
//! It **browses and composes the command**; it does not yet collect a flag's
//! value interactively, so an operation with required inputs shows its command
//! with the flags named and hands it to the operator to run. That is stated
//! rather than hidden (`AGENTS.md` §3), it is shown on the screen itself, and
//! it is the honest half: what is built is `ADR-034` §4.1's *structure* — one
//! operation set, two renderings — and what is not is a form.

use std::io::{self, Stdout};

pub mod device;
pub mod edit;
pub mod fleet;
pub mod log;
pub mod reach;
pub mod theme;
pub mod top;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{ExecutableCommand as _, event::KeyModifiers};

use crate::operation::{CONFIRMATION, Operation};

/// The command line an operation is spelled as, exactly as `clap` takes it.
///
/// **Generated from the same table `crate::cli` builds the parser from**, which
/// is what makes §4.2's promise true rather than approximately true: this is
/// the command, not a rendering of one. A placeholder appears where a value
/// would go, in angle brackets, so it is obvious what the operator must supply.
#[must_use]
// **Unused since the fleet screen replaced the operation browser**, and kept
// rather than deleted: the device screen shows the command each change would
// run, which is `ADR-006` §3.1's conformance property made visible to a person
// — the terminal interface and the command line doing the same thing in the
// same words. Allowed explicitly so the warning does not sit there masking a
// real one.
#[allow(dead_code)]
pub fn command_line(operation: &Operation) -> String {
    let mut out = format!("rivet-fleet {} {}", operation.verb.as_str(), operation.subject);
    for input in operation.inputs {
        if input.required {
            out.push_str(&format!(" --{} <{}>", input.name, input.name));
        }
    }
    if operation.confirms {
        out.push_str(&format!(" --{CONFIRMATION}"));
    }
    out
}

/// Every optional flag, for the pane that explains the selection.
#[must_use]
#[allow(dead_code)]
pub fn optional_flags(operation: &Operation) -> Vec<String> {
    operation
        .inputs
        .iter()
        .filter(|input| !input.required)
        .map(|input| format!("--{} <{}>", input.name, input.name))
        .collect()
}

/// Run the interface until the operator leaves it.
///
/// # Errors
///
/// Whatever the terminal refused. **The terminal is restored first**, in every
/// path including the error one: a tool that left a workstation's shell in raw
/// mode after failing would have done more damage than the failure.
/// How often the loop wakes when nothing has happened.
///
/// **250 ms**, which is short enough that a probe's answer appears as soon as it
/// lands and long enough that an idle interface is not a busy loop. It is a
/// poll rather than a wait-on-many because there are exactly two things to
/// watch — the keyboard and the probe channel — and `crossterm` can wait on the
/// first with a deadline.
const TICK: std::time::Duration = std::time::Duration::from_millis(250);

/// How often a following journal re-asks.
///
/// **Two seconds.** A round trip is 213 ms since `PLAN.md` §51, so this is
/// roughly a tenth of the connection budget and still feels live. It is not
/// faster because every tick is a connection, and `crate::tui::log` refuses a
/// second fetch while one is in flight rather than letting them pile up.
const FOLLOW_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// Which screen the interface is on.
///
/// **A device is held by identifier inside `Device`**, never as an index into
/// the list — `crate::tui::device` carries why, and the short form is that a
/// probe answering or a device being hidden must not silently change which
/// machine a person is looking at.
enum Screen {
    Fleet,
    Device(device::DeviceScreen),
}

/// What one save did, for the line under the screen.
///
/// **Said out loud and kept until the next thing happens**, because a person
/// who pressed `A` and saw nothing change would not know whether it worked.
struct Said {
    text: String,
    bad: bool,
}

/// Everything on screen, and everything it is waiting for.
struct App {
    screen: Screen,
    /// Edits made and not yet saved. **One device's**, abandoned on leaving —
    /// `crate::tui::edit::Draft` carries why.
    draft: edit::Draft,
    /// The question on screen, if one is being asked.
    confirm: Option<edit::Confirm>,
    /// What the last save did.
    said: Option<Said>,
    /// Journal fetches in flight — `crate::tui::log`.
    logs: log::Fetches,
    /// When the journal was last asked, for the follow timer.
    last_fetch: std::time::Instant,
    register: crate::register::Register,
    fleet: fleet::Fleet,
    reach: std::collections::BTreeMap<String, reach::Reach>,
    /// **Why**, for the states where why is the whole of the news.
    ///
    /// Kept beside [`Self::reach`] rather than in it, because the fleet list
    /// draws only the word and copying a string per row per draw to render a
    /// glyph would be paying for something no row shows. The device screen is
    /// where it is read.
    reach_detail: std::collections::BTreeMap<String, String>,
    probes: reach::Probes,
    /// **The monitor's reader, and which device it belongs to.**
    ///
    /// One at a time, and dropped when a person leaves the device — which ends
    /// the worker thread. It lives here rather than on `device::DeviceScreen`
    /// for two reasons: walking the rail must not create and destroy a reader,
    /// and the identifier beside it is what makes an answer for a machine
    /// nobody is looking at any more impossible to render (`crate::tui::device`
    /// keeps a screen by identifier for the same reason).
    monitor: Option<(String, crate::telemetry::Poller)>,
    /// **Why there is no reader**, when there is none and a person is looking.
    ///
    /// `crate::operator`'s own words, kept rather than dropped: *no identity was
    /// given* and *the pane has not been opened yet* are different states and a
    /// screen that showed one sentence for both would be guessing at which.
    monitor_refusal: Option<String>,
    /// **This operator's identity**, for the whole run — `ADR-034` §11.5.2.
    operator: crate::operator::Operator,
    glyphs: theme::Glyphs,
    palette: theme::Palette,
    /// **`ADR-052` §9 step 2's fleet repository, resolved once by [`run`] and
    /// carried for the run** — the same pattern `operator` above already
    /// uses. [`save`] hands it to `crate::save` exactly as every handler in
    /// `main.rs` hands its own `project_root` to `crate::save`.
    project_root: Option<std::path::PathBuf>,
}

impl App {
    fn load(
        register_root: &std::path::Path,
        operator: crate::operator::Operator,
        project_root: Option<std::path::PathBuf>,
    ) -> io::Result<Self> {
        // A register that will not load is a reason to refuse to start, not to
        // start empty: an empty screen and a broken register look identical,
        // and one of them is a person's fleet.
        let register = crate::store::load(register_root)
            .map_err(|e| io::Error::other(e.to_string()))?;
        Ok(Self {
            screen: Screen::Fleet,
            draft: edit::Draft::for_device(""),
            confirm: None,
            said: None,
            logs: log::Fetches::new(),
            last_fetch: std::time::Instant::now(),
            register,
            fleet: fleet::Fleet::new(),
            reach: std::collections::BTreeMap::new(),
            reach_detail: std::collections::BTreeMap::new(),
            probes: reach::Probes::new(),
            monitor: None,
            monitor_refusal: None,
            operator,
            glyphs: theme::Glyphs::from_env(),
            palette: theme::Palette::from_env(),
            project_root,
        })
    }

    /// Ask every device whether it answers.
    ///
    /// **Once, at start-up, and every device at the same time.** Each probe is
    /// its own thread and none of them blocks the draw, so a fleet of dead
    /// machines costs one timeout rather than one timeout each.
    fn probe_everything(&self) {
        for device in self.register.devices.values() {
            // **The transport is built here and the probe is handed it**, so
            // that a run with no identity, or a device whose record holds no
            // pin, is `Reach::Unasked` with the reason attached rather than a
            // row that silently never resolves.
            match self.operator.open(device) {
                Ok(transport) => self.probes.ask(&device.id, transport),
                Err(why) => self.probes.cannot_ask(&device.id, why),
            }
        }
    }

    /// Take whatever the probes have answered. Never blocks.
    fn collect(&mut self) {
        for answer in self.probes.collected() {
            self.reach.insert(answer.id.clone(), answer.reach);
            // **Removed when there is nothing to say**, so a machine that came
            // back does not keep the sentence explaining why it was away.
            match answer.detail {
                Some(why) => self.reach_detail.insert(answer.id, why),
                None => self.reach_detail.remove(&answer.id),
            };
        }
        for (id, fetched) in self.logs.collected() {
            // **Only if the screen is still about that machine.** An answer for
            // a device a person has navigated away from is not theirs any more.
            if let Screen::Device(open) = &mut self.screen
                && open.id == id
            {
                open.log = Some(fetched);
            }
        }
    }

    /// Ask the open device for its journal.
    fn fetch_log(&mut self) {
        let Screen::Device(open) = &self.screen else {
            return;
        };
        let Some(device) = self.register.devices.get(&open.id) else {
            return;
        };
        self.last_fetch = std::time::Instant::now();
        match self.operator.open(device) {
            Ok(transport) => self.logs.ask(&open.id, transport),
            // **Said rather than swallowed.** This used to return silently when
            // the record carried no address, which left the pane showing
            // nothing and saying nothing about why.
            Err(why) => self.logs.cannot_ask(&open.id, why),
        }
    }

    /// Re-ask, if the journal is following and it is time.
    ///
    /// **A poll, because `--follow` never reaches the operation catalogue** —
    /// `crate::tui::log`'s header carries the whole of why, and why changing
    /// that is a ruling rather than a fix.
    fn follow_tick(&mut self) {
        let following = matches!(&self.screen, Screen::Device(open) if open.following);
        if following && self.last_fetch.elapsed() >= FOLLOW_EVERY {
            self.fetch_log();
        }
    }

    /// Keep the monitor's reader in step with what is on screen.
    ///
    /// **Three rules, and the second one is the whole of `crate::telemetry`'s
    /// property 2:**
    ///
    /// 1. A reader exists only for the device the screen is about. Leaving the
    ///    device drops it, which ends its worker thread.
    /// 2. **It only asks while the monitor pane is the one being looked at.**
    ///    An answer already in flight is still collected — throwing it away
    ///    would mean paying for a request and not using it — but nothing new is
    ///    sent.
    /// 3. It is created when the pane is first opened, not when the device is,
    ///    so a person who never looks at the monitor never opens a connection.
    fn monitor_tick(&mut self) {
        let (want, active) = match &self.screen {
            Screen::Device(open) => (
                Some(open.id.clone()),
                open.pane == device::Pane::Monitor,
            ),
            Screen::Fleet => (None, false),
        };

        // A reader for a machine that is no longer on screen is dropped rather
        // than paused: the person has left, and its worker has nothing to say
        // that anyone will read.
        if self.monitor.as_ref().is_some_and(|(id, _)| Some(id) != want.as_ref()) {
            self.monitor = None;
            self.monitor_refusal = None;
        }
        let Some(id) = want else {
            return;
        };
        if !active && self.monitor.is_none() {
            return;
        }
        if self.monitor.is_none() {
            // **Refused once is refused**, until the person leaves the device.
            // Nothing that would change the answer can change while this screen
            // is open — the identity is a flag of this run — so retrying every
            // 250 ms would re-read a key file forty times a second to compose
            // the same sentence.
            if self.monitor_refusal.is_some() {
                return;
            }
            let Some(device) = self.register.devices.get(&id) else {
                return;
            };
            // **The control plane, and no other route** — `ADR-034` §11.5.1.
            // Whatever stopped it is kept and shown by the pane in words rather
            // than swallowed: a monitor that silently never started is exactly
            // the *dead pane that looks like a still one* this whole feature is
            // about.
            match self.operator.open(device) {
                Ok(transport) => {
                    self.monitor_refusal = None;
                    self.monitor = Some((
                        id,
                        crate::telemetry::Poller::spawn(Box::new(crate::telemetry::Tls {
                            transport,
                        })),
                    ));
                }
                Err(why) => {
                    self.monitor_refusal = Some(why);
                    return;
                }
            }
        }
        if let Some((_, poller)) = &mut self.monitor {
            poller.collect();
            poller.tick(std::time::Instant::now(), active);
        }
    }

    /// The reader for the device on screen, if it is this one's.
    fn monitor_of(&self, id: &str) -> Option<&crate::telemetry::Poller> {
        self.monitor
            .as_ref()
            .filter(|(open, _)| open == id)
            .map(|(_, poller)| poller)
    }
}

pub fn run(
    register_root: &std::path::Path,
    operator: crate::operator::Operator,
    project_root: Option<std::path::PathBuf>,
) -> io::Result<()> {
    let mut app = App::load(register_root, operator, project_root)?;

    let mut out = io::stdout();
    enable_raw_mode()?;
    out.execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;

    app.probe_everything();
    let outcome = main_loop(&mut terminal, &mut app);

    // **Restored whatever happened.** A program that leaves a terminal in raw
    // mode has taken the machine away from the person using it, which is worse
    // than any error it could have been reporting.
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    outcome
}

fn main_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        app.collect();
        app.follow_tick();
        app.monitor_tick();
        // **A device that vanished sends the interface back**, rather than
        // leaving a screen about nothing on the terminal. A register can be
        // reloaded while this screen is open.
        if let Screen::Device(open) = &app.screen
            && open.device(&app.register).is_none()
        {
            app.screen = Screen::Fleet;
        }

        terminal.draw(|frame| {
            match &app.screen {
            Screen::Fleet => fleet::draw(
                frame,
                frame.area(),
                &app.fleet,
                &app.register,
                &app.reach,
                app.glyphs,
                app.palette,
            ),
            Screen::Device(open) => {
                // Checked immediately above, so this cannot be absent — and it
                // is still written as a match rather than an unwrap, because a
                // panic here takes the terminal down with it.
                if let Some(device) = open.device(&app.register) {
                    let reach = app.reach.get(&open.id).copied().unwrap_or(reach::Reach::Unknown);
                    device::draw(
                        frame,
                        frame.area(),
                        open,
                        device,
                        &app.draft,
                        reach,
                        app.reach_detail.get(&open.id).map(String::as_str),
                        &app.register.fleet,
                        app.glyphs,
                        app.palette,
                        app.monitor_of(&open.id),
                        app.monitor_refusal.as_deref(),
                        std::time::Instant::now(),
                    );
                }
            }
            }
            if let Some(confirm) = &app.confirm {
                device::draw_confirm(frame, frame.area(), confirm, app.palette);
            }
            if let Some(said) = &app.said {
                let style = if said.bad { app.palette.error() } else { app.palette.up() };
                let line = ratatui::layout::Rect {
                    x: frame.area().x,
                    y: frame.area().bottom().saturating_sub(1),
                    width: frame.area().width,
                    height: 1,
                };
                frame.render_widget(
                    ratatui::widgets::Paragraph::new(ratatui::text::Line::from(
                        ratatui::text::Span::styled(format!(" {}", said.text), style),
                    )),
                    line,
                );
            }
        })?;

        // Wait for a key, but not for ever: a probe's answer has to reach the
        // screen without the person pressing anything.
        if !event::poll(TICK)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if handle(key, app) == Flow::Leave {
            return Ok(());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Stay,
    Leave,
}

fn handle(key: event::KeyEvent, app: &mut App) -> Flow {
    // Ctrl-C leaves from anywhere, because it is what a person reaches for
    // when a program is not doing what they expected.
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Flow::Leave;
    }
    // **And no OTHER control chord does anything at all**, which was not true
    // until 2026-08-29 and was found by running the interface rather than by
    // reading it.
    //
    // A terminal sends `Ctrl-D` as the byte `0x04`, and crossterm reports that
    // as `KeyCode::Char('d')` with `KeyModifiers::CONTROL`. Every match below
    // is on the *code* alone, so **Ctrl-D did what `d` does**: it cycled the
    // monitor's refresh interval. Measured in a pty — the banner went from
    // *every 2s* to *every 3s* at the frame after the byte, with nobody having
    // pressed `d`. `Ctrl-R` re-armed every subject and `Ctrl-P` would have
    // paused the reader; `Ctrl-K` would have scrolled the process table.
    //
    // None of that is catastrophic and all of it is wrong: a chord is not the
    // letter in it, and a person who pressed `Ctrl-D` meant end-of-input.
    // Swallowed rather than dispatched, and swallowed HERE rather than at each
    // match, because the next key added would inherit the same defect.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return Flow::Stay;
    }
    // **A question on screen takes every key**, so nothing behind it moves
    // while it is being answered.
    if app.confirm.is_some() {
        return on_confirm(key, app);
    }
    // Anything a person does clears the last answer, because a sentence about
    // something they have moved on from is noise.
    app.said = None;
    match &mut app.screen {
        Screen::Fleet => on_fleet(key, app),
        Screen::Device(_) => on_device(key, app),
    }
}

/// **Escape goes back one screen, and only leaves from the top.** A key that
/// quits the program from three screens deep is how a person loses their place.
/// The dialog. **`A` or `L`, `Esc` cancels, and nothing is focused** — ruled
/// by the maintainer 2026-08-27, so `enter` is not an answer.
fn on_confirm(key: event::KeyEvent, app: &mut App) -> Flow {
    let answer = match key.code {
        KeyCode::Esc => edit::Answer::Cancel,
        KeyCode::Char(c) => edit::answer(c),
        _ => edit::Answer::Nothing,
    };
    match answer {
        // The draft survives a cancel, so a person who hit the wrong key has
        // not lost their edits.
        edit::Answer::Cancel => app.confirm = None,
        edit::Answer::Nothing => {}
        edit::Answer::Later => {
            app.said = Some(save(app));
            app.confirm = None;
        }
        edit::Answer::ApplyNow => {
            let mut said = save(app);
            if !said.bad {
                said = apply_now(app, said);
            }
            app.said = Some(said);
            app.confirm = None;
        }
    }
    Flow::Stay
}

/// Write the draft into the register.
fn save(app: &mut App) -> Said {
    let id = app.draft.device.clone();
    let Some(device) = app.register.devices.get_mut(&id) else {
        return Said {
            text: "that device is no longer in the register; nothing was written".to_string(),
            bad: true,
        };
    };
    let count = app.draft.changes(device).len();
    app.draft.apply_to(device);
    // `crate::save`, not `store::save` — `ADR-052` §9 step 2: every write of
    // the whole register, here as much as in `main.rs`'s own handlers, also
    // projects the same state into the fleet repository, best-effort.
    // `app.project_root` is `App::load`'s copy of the same root [`run`]'s
    // caller resolved once in `main`.
    match crate::save(&app.register, app.project_root.as_deref()) {
        Ok(_written) => {
            app.draft = edit::Draft::for_device(&id);
            Said {
                text: format!("saved {count} change(s) to the register"),
                bad: false,
            }
        }
        // **The draft is kept on a failed write**, so the edits are not lost
        // with the file that would not take them.
        Err(e) => Said {
            text: format!("the register would not take it: {e}"),
            bad: true,
        },
    }
}

/// Tell the machine.
///
/// **Synchronous, and that is a known wart.** `apply document` opens a
/// connection and the interface stops until it answers — which is bearable now
/// only because `PLAN.md` §51 took a connection from 5.2 s to 213 ms. It should
/// be a thread like `crate::tui::reach`'s probes, and is not one yet.
///
/// # It is a pause, and it cannot become a hang
///
/// A transport that asks the operator a question — an unknown host, a
/// passphrase, a password — would ask it *through* this interface: painted over
/// the alternate screen, reading the operator's key presses as its answer, and
/// waiting for ever. That is worth a reader knowing about, because the
/// mitigation is structural rather than defensive.
///
/// **`ADR-035` §7.2's control plane has nothing to prompt for.** The identity is
/// a directory named on the command line and the trust is a pin out of the
/// register, so there is no interactive question for a TLS client to ask and no
/// terminal for it to ask on.
fn apply_now(app: &mut App, saved: Said) -> Said {
    let id = app.draft.device.clone();
    let id = if id.is_empty() {
        match &app.screen {
            Screen::Device(open) => open.id.clone(),
            Screen::Fleet => return saved,
        }
    } else {
        id
    };
    let Some(device) = app.register.devices.get(&id) else {
        return saved;
    };
    let mut remote = match app.operator.open(device) {
        Ok(transport) => transport,
        Err(e) => {
            return Said {
                text: format!("{e} — saved, not applied"),
                bad: true,
            };
        }
    };
    let mut report = String::new();
    match crate::member::put_document(device, &app.register.fleet, &mut remote, true, &mut report)
    {
        Ok(_outcome) => Said {
            text: format!("{} — applied to {}", saved.text, device.name),
            bad: false,
        },
        // **Saved is still true.** A machine that would not take it does not
        // undo the register, and saying only the failure would hide that.
        Err(e) => Said {
            text: format!("{} — but the machine did not take it: {e}", saved.text),
            bad: true,
        },
    }
}

fn on_device(key: event::KeyEvent, app: &mut App) -> Flow {
    // **The reader's keys are taken first, and before the screen is borrowed.**
    // They change the *reader* — how often it asks, whether it asks at all —
    // which lives on the application rather than on the screen, and the two
    // borrows cannot be held at once.
    let on_monitor =
        matches!(&app.screen, Screen::Device(open) if open.pane == device::Pane::Monitor);
    if on_monitor && matches!(key.code, KeyCode::Char('d' | 'p' | 'r')) {
        if let Some((_, poller)) = &mut app.monitor {
            match key.code {
                // `top`'s key for the refresh delay.
                KeyCode::Char('d') => poller.cycle_fast_interval(),
                KeyCode::Char('p') => poller.paused = !poller.paused,
                // **Everything, including the subjects this build was told it
                // does not have** — a board updated since the pane was opened
                // now answers them, and there is no other way to find out.
                _ => poller.refresh_all(),
            }
        }
        // Swallowed even when there is no reader yet, so a key the footer
        // advertises never does something else instead.
        return Flow::Stay;
    }

    let Screen::Device(open) = &mut app.screen else {
        return Flow::Stay;
    };
    // **The filter takes every key while it is being typed**, or `q` would
    // quit in the middle of a word.
    if open.pane == device::Pane::Journal && open.filtering {
        match key.code {
            KeyCode::Esc => {
                open.filter.clear();
                open.filtering = false;
            }
            KeyCode::Enter => open.filtering = false,
            KeyCode::Backspace => {
                open.filter.pop();
            }
            KeyCode::Char(c) => open.filter.push(c),
            _ => {}
        }
        return Flow::Stay;
    }

    // **The monitor's keys, before the rail's.** Up and down scroll the process
    // table here rather than stepping the rail: a pane with a list in it should
    // not swallow the keys that move through the list, which is the same bargain
    // the Services pane already makes.
    if on_monitor {
        let handled = match key.code {
            KeyCode::Char('K') => {
                open.monitor.kernel_threads = !open.monitor.kernel_threads;
                open.monitor.scroll = 0;
                true
            }
            KeyCode::Char('o') => {
                open.monitor.order = open.monitor.order.next();
                open.monitor.scroll = 0;
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                // **Capped, because the key handler cannot see the pane's
                // height and the draw is what clamps to the real end.** Without
                // a cap, a held key runs the counter into the thousands and the
                // first several hundred presses of `up` then appear to do
                // nothing. `crate::tui::top::SCROLL_LIMIT` is the number.
                open.monitor.scroll = open
                    .monitor
                    .scroll
                    .saturating_add(1)
                    .min(top::SCROLL_LIMIT);
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                open.monitor.scroll = open.monitor.scroll.saturating_sub(1);
                true
            }
            KeyCode::Home => {
                open.monitor.scroll = 0;
                true
            }
            _ => false,
        };
        if handled {
            return Flow::Stay;
        }
    }

    let on_services = open.pane == device::Pane::Services;
    let on_journal = open.pane == device::Pane::Journal;
    if on_journal {
        match key.code {
            KeyCode::Char('g') => {
                app.fetch_log();
                return Flow::Stay;
            }
            KeyCode::Char('f') => {
                open.following = !open.following;
                if open.following {
                    app.fetch_log();
                }
                return Flow::Stay;
            }
            KeyCode::Char('/') => {
                open.filtering = true;
                return Flow::Stay;
            }
            KeyCode::Char('w') => {
                open.trouble_only = !open.trouble_only;
                return Flow::Stay;
            }
            _ => {}
        }
    }
    match key.code {
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
            // **Leaving abandons the draft**, and the dialog is how a person
            // keeps it. That is the same bargain `esc` makes everywhere else in
            // this interface: it undoes, it does not defer.
            app.draft = edit::Draft::for_device("");
            app.screen = Screen::Fleet;
        }
        KeyCode::Char('q') => return Flow::Leave,
        KeyCode::Tab => open.step_pane(1),
        KeyCode::BackTab => open.step_pane(-1),
        // Within a pane that has rows, up and down move the row; elsewhere they
        // move the rail, because a pane with nothing to point at should not
        // swallow the keys that navigate.
        KeyCode::Down | KeyCode::Char('j') => {
            if on_services {
                open.row = open.row.saturating_add(1);
            } else {
                open.step_pane(1);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if on_services {
                open.row = open.row.saturating_sub(1);
            } else {
                open.step_pane(-1);
            }
        }
        KeyCode::Char(' ') if on_services => {
            let id = open.id.clone();
            let row = open.row;
            if let Some(device) = app.register.devices.get(&id) {
                let names = device::service_names(device);
                if let Some(name) = names.get(row) {
                    if app.draft.device != id {
                        app.draft = edit::Draft::for_device(&id);
                    }
                    app.draft.toggle_service(device, name);
                }
            }
        }
        KeyCode::Char('s') => {
            let id = open.id.clone();
            if let Some(device) = app.register.devices.get(&id) {
                let changes = app.draft.changes(device);
                if changes.is_empty() {
                    app.said = Some(Said {
                        text: "nothing to save".to_string(),
                        bad: false,
                    });
                } else {
                    app.confirm = Some(edit::Confirm {
                        device: device.name.clone(),
                        changes,
                    });
                }
            }
        }
        _ => {}
    }
    Flow::Stay
}

fn on_fleet(key: event::KeyEvent, app: &mut App) -> Flow {
    match key.code {
        // `q` and Escape both leave, because a full-screen program a person
        // cannot get out of is the one thing worse than no program.
        KeyCode::Char('q') | KeyCode::Esc => return Flow::Leave,
        // **What the footer has promised since the fleet screen was written.**
        // It said `[↵] open` and nothing happened, which is the same defect as
        // a switch that does not switch.
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if let Some(device) = app.fleet.selected(&app.register) {
                app.screen = Screen::Device(device::DeviceScreen::opening(&device.id));
            }
        }
        KeyCode::Down | KeyCode::Char('j') => app.fleet.step(1, &app.register),
        KeyCode::Up | KeyCode::Char('k') => app.fleet.step(-1, &app.register),
        KeyCode::Char(' ') => app.fleet.toggle_mark(&app.register),
        KeyCode::Char('H') => app.fleet.show_hidden = !app.fleet.show_hidden,
        // Re-probe. A person who has just switched a board on should not have
        // to restart the interface to see it come up.
        KeyCode::Char('r') => {
            app.reach.clear();
            app.probe_everything();
        }
        _ => {}
    }
    Flow::Stay
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **Two ways out, and one of them is the one every terminal user tries
    /// first.** A full-screen program a person cannot leave is worse than no
    /// program, and this is the assertion that keeps saying so.
    #[test]
    fn there_is_always_a_way_out() {
        let source = include_str!("mod.rs");
        let body = source.split("#[cfg(test)]").next().expect("a body");
        assert!(body.contains("KeyCode::Char('q') | KeyCode::Esc"));
        assert!(body.contains("KeyModifiers::CONTROL"));
    }

    /// **A chord is not the letter in it**, and until 2026-08-29 it was.
    ///
    /// `Ctrl-D` arrives as `KeyCode::Char('d')` with `CONTROL`, and every match
    /// in this file is on the code alone — so it cycled the monitor's refresh
    /// interval. Found by driving the interface in a pty, where the banner went
    /// from *every 2s* to *every 3s* with nobody having pressed `d`.
    ///
    /// This asserts the *shape* of the fix rather than the behaviour, for the
    /// same reason [`there_is_always_a_way_out`] does: `handle` needs a whole
    /// `App` and a `KeyEvent` to call, and the property worth guarding is that
    /// the control-chord gate stands **before** the dispatch rather than that
    /// one particular key is safe today.
    #[test]
    fn a_control_chord_reaches_no_key_handler_except_the_way_out() {
        let source = include_str!("mod.rs").replace("\r\n", "\n");
        let body = source.split("#[cfg(test)]").next().expect("a body");
        let handle = body
            .split("fn handle(key: event::KeyEvent")
            .nth(1)
            .expect("the dispatcher is in this file");
        let gate = handle
            .find("if key.modifiers.contains(KeyModifiers::CONTROL) {\n        return Flow::Stay;")
            .expect("control chords are swallowed before anything dispatches");
        let dispatch = handle.find("match &mut app.screen").expect("the dispatch");
        assert!(
            gate < dispatch,
            "the control-chord gate is after the dispatch, so a chord reaches a handler again"
        );
        // And Ctrl-C is still ahead of the gate, or the way out would be gone.
        let leave = handle.find("return Flow::Leave;").expect("the way out");
        assert!(leave < gate, "Ctrl-C is now swallowed with everything else");
    }

    /// **The terminal is restored whatever happened**, including on the error
    /// path — a program that leaves raw mode on has taken the machine away.
    #[test]
    fn the_terminal_is_restored_before_the_outcome_is_returned() {
        // **Normalised first.** This repository has no `.gitattributes` entry
        // for `*.rs`, so a Windows checkout has CRLF and `"outcome\n}"` is not
        // in the file — the assertion below failed for a reason that had
        // nothing to do with the property it guards. Found 2026-08-29 while
        // adding the monitor pane; the test was red on the maintainer's own
        // machine and green in CI, which is the worst of the two.
        let source = include_str!("mod.rs").replace("\r\n", "\n");
        let restore = source.find("disable_raw_mode()?").expect("restored");
        let outcome = source.find("outcome\n}").expect("returned");
        assert!(restore < outcome, "the outcome is returned before the restore");
    }

    /// The loop wakes on its own, so an answer from a probe reaches the screen
    /// without the person pressing a key.
    #[test]
    fn the_loop_does_not_wait_for_a_keypress_for_ever() {
        assert!(TICK.as_millis() > 0 && TICK.as_millis() <= 1000);
        let source = include_str!("mod.rs");
        assert!(
            source.contains("event::poll(TICK)"),
            "the loop must wait with a deadline, not block on a key"
        );
    }
}
