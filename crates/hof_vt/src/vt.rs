//! libghostty-vt's terminal, the encoders that answer for it, and the
//! callbacks that turn what it parses into [`Changes`] and [`Event`]s.

use std::sync::{Arc, Mutex};

use libghostty_vt::Terminal;
use libghostty_vt::key;
use libghostty_vt::mouse;
use libghostty_vt::screen::TrackedGridRef;
use libghostty_vt::terminal::{
    ColorScheme, ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType,
    PrimaryDeviceAttributes, ProgressState, SecondaryDeviceAttributes, SemanticPromptKind,
    SizeReportSize, TertiaryDeviceAttributes,
};

use crate::signals::{Changes, Event, Progress, PromptMark, Signals};
use crate::{Error, Size};

/// Where the terminal's replies go: the program's input, or a buffer for a
/// detached session.
#[derive(Clone)]
pub(crate) enum Sink {
    Pty(std::sync::mpsc::Sender<Vec<u8>>),
    Buffer(Arc<Mutex<Vec<u8>>>),
    /// A remote session's connection, once it has one.
    Outlet(Arc<crate::remote::Slot>),
}

impl Sink {
    pub fn write(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        match self {
            // A closed channel means the program is gone; its input with it.
            Sink::Pty(tx) => drop(tx.send(bytes.to_vec())),
            Sink::Buffer(buf) => buf
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend_from_slice(bytes),
            Sink::Outlet(slot) => {
                if let Some(outlet) = slot.get() {
                    outlet.input(bytes);
                }
            }
        }
    }
}

/// How a drag extends a selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Selecting {
    #[default]
    Cells,
    Words,
    Lines,
    /// A block of columns, as Alt-drag does elsewhere.
    Block,
}

pub(crate) struct Anchor {
    pub at: TrackedGridRef,
    pub unit: Selecting,
}

/// Sees every chunk of the program's output, just after it is parsed.
pub type Tap = Box<dyn FnMut(&[u8]) + Send>;

pub(crate) struct Vt {
    /// Called with the program's output under the same lock that parses
    /// it, so what it has seen is always what the screen shows.
    pub tap: Option<Tap>,
    /// What a find bar is looking for, if anything. Before the terminal so
    /// it lets go of the terminal first.
    pub finder: Option<crate::find::Finder>,
    /// The line a command was typed on, from `mark_command`: where its
    /// output is read from when it ends. Before the terminal, as above.
    pub command_mark: Option<TrackedGridRef>,
    pub term: Terminal<'static, 'static>,
    pub keys: key::Encoder<'static>,
    pub key_event: key::Event<'static>,
    pub mouse: mouse::Encoder<'static>,
    pub mouse_event: mouse::Event<'static>,
    pub mouse_buttons: u8,
    pub size: Size,
    pub anchor: Option<Anchor>,
    pub out: Vec<u8>,
    /// Picks Hof's agent reports out of the output, which libghostty
    /// passes over.
    reports: crate::report::Scanner,
    signals: Arc<Signals>,
}

// SAFETY: libghostty's objects are not `Send` only because they hold raw
// pointers. They keep no thread-local state, and a `Vt` is only ever reached
// through the session's mutex, so at most one thread touches it at a time.
// The callbacks run synchronously inside calls that already hold that lock,
// and capture only `Send + Sync` state of their own.
unsafe impl Send for Vt {}

/// What the terminal says about itself to programs that ask.
#[derive(Clone)]
pub(crate) struct Identity {
    pub scheme: Arc<std::sync::atomic::AtomicBool>,
    /// Whether the terminal's replies reach the program; a paste goes
    /// through the same callback, so it turns this on for its moment.
    pub replies: Arc<std::sync::atomic::AtomicBool>,
}

impl Identity {
    /// What `options` start it as.
    pub fn of(options: &crate::Options) -> Self {
        use std::sync::atomic::AtomicBool;
        Self {
            scheme: Arc::new(AtomicBool::new(options.dark)),
            replies: Arc::new(AtomicBool::new(options.replies)),
        }
    }
}

impl Vt {
    pub fn new(
        size: Size,
        scrollback: usize,
        term_name: &str,
        sink: Sink,
        signals: Arc<Signals>,
        identity: Identity,
    ) -> Result<Self, Error> {
        let size = size.clamped();
        let mut term = Terminal::new(size.cols, size.rows)?;
        term.set_scrollback_max_lines(Some(scrollback))?;
        term.set_terminfo_name(term_name)?;
        term.resize(size.cols, size.rows, size.cell_width, size.cell_height)?;
        install_callbacks(&mut term, sink, signals.clone(), identity)?;
        Ok(Self {
            tap: None,
            finder: None,
            command_mark: None,
            term,
            keys: key::Encoder::new()?,
            key_event: key::Event::new()?,
            mouse: mouse::Encoder::new()?,
            mouse_event: mouse::Event::new()?,
            mouse_buttons: 0,
            size,
            anchor: None,
            out: Vec::with_capacity(64),
            reports: crate::report::Scanner::default(),
            signals,
        })
    }

    /// Parses the program's output, and hears what it reports of its agent.
    pub fn output(&mut self, bytes: &[u8]) {
        self.term.vt_write(bytes);
        let signals = &self.signals;
        self.reports
            .scan(bytes, |report| signals.push(Event::Agent(report)));
    }

    pub fn resize(&mut self, size: Size) -> Result<(), Error> {
        let size = size.clamped();
        self.term
            .resize(size.cols, size.rows, size.cell_width, size.cell_height)?;
        self.size = size;
        Ok(())
    }

    /// Encode a key for the program, per the keyboard modes it set.
    pub fn encode_key(&mut self, input: &crate::KeyInput) -> Result<&[u8], Error> {
        let action = match (input.release, input.repeat) {
            (true, _) => key::Action::Release,
            (false, true) => key::Action::Repeat,
            (false, false) => key::Action::Press,
        };
        self.key_event
            .set_action(action)
            .set_key(input.key)
            .set_mods(input.mods)
            .set_consumed_mods(input.consumed)
            .set_composing(input.composing)
            .set_utf8(input.text.clone())
            .set_unshifted_codepoint(input.unshifted.unwrap_or('\0'));
        self.keys.set_options_from_terminal(&self.term);
        self.out.clear();
        self.keys.encode_to_vec(&self.key_event, &mut self.out)?;
        Ok(&self.out)
    }

    /// Encode a mouse event, if the program is tracking the mouse.
    pub fn encode_mouse(&mut self, input: &crate::MouseInput) -> Result<&[u8], Error> {
        use crate::{MouseAction, MouseButton};
        let action = match input.action {
            MouseAction::Press => mouse::Action::Press,
            MouseAction::Release => mouse::Action::Release,
            MouseAction::Motion => mouse::Action::Motion,
        };
        let wheel = matches!(
            input.button,
            Some(
                MouseButton::WheelUp
                    | MouseButton::WheelDown
                    | MouseButton::WheelLeft
                    | MouseButton::WheelRight
            )
        );
        if !wheel {
            match input.action {
                MouseAction::Press => self.mouse_buttons = self.mouse_buttons.saturating_add(1),
                MouseAction::Release => self.mouse_buttons = self.mouse_buttons.saturating_sub(1),
                MouseAction::Motion => {}
            }
        }
        let button = input.button.map(|b| match b {
            MouseButton::Left => mouse::Button::Left,
            MouseButton::Middle => mouse::Button::Middle,
            MouseButton::Right => mouse::Button::Right,
            MouseButton::WheelUp => mouse::Button::Four,
            MouseButton::WheelDown => mouse::Button::Five,
            MouseButton::WheelLeft => mouse::Button::Six,
            MouseButton::WheelRight => mouse::Button::Seven,
            MouseButton::Back => mouse::Button::Eight,
            MouseButton::Forward => mouse::Button::Nine,
        });
        let s = self.size;
        self.mouse
            .set_options_from_terminal(&self.term)
            .set_size(mouse::EncoderSize {
                screen_width: u32::from(s.cols) * s.cell_width,
                screen_height: u32::from(s.rows) * s.cell_height,
                cell_width: s.cell_width,
                cell_height: s.cell_height,
                padding_top: 0,
                padding_bottom: 0,
                padding_right: 0,
                padding_left: 0,
            })
            .set_any_button_pressed(self.mouse_buttons > 0);
        self.mouse_event
            .set_action(action)
            .set_button(button)
            .set_mods(input.mods)
            .set_position(mouse::Position {
                x: input.x,
                y: input.y,
            });
        self.out.clear();
        self.mouse.encode_to_vec(&self.mouse_event, &mut self.out)?;
        Ok(&self.out)
    }
}

fn install_callbacks(
    term: &mut Terminal<'static, 'static>,
    sink: Sink,
    signals: Arc<Signals>,
    identity: Identity,
) -> Result<(), Error> {
    let replies = identity.replies.clone();
    term.on_pty_write(move |_, bytes| {
        if replies.load(std::sync::atomic::Ordering::Relaxed) {
            sink.write(bytes);
        }
    })?;

    let s = signals.clone();
    term.on_bell(move |_| s.mark(Changes::BELL))?;
    let s = signals.clone();
    term.on_title_changed(move |_| s.mark(Changes::TITLE))?;
    let s = signals.clone();
    term.on_pwd_changed(move |_| s.mark(Changes::PWD))?;
    let s = signals.clone();
    term.on_render_hold(move |_, held| {
        s.set_held(held);
        s.mark(Changes::SCREEN);
    })?;

    let s = signals.clone();
    term.on_desktop_notification(move |_, n| {
        s.push(Event::Notification {
            title: String::from_utf8_lossy(n.title()).into_owned(),
            body: String::from_utf8_lossy(n.body()).into_owned(),
        });
    })?;

    let s = signals.clone();
    term.on_progress_report(move |_, report| {
        let value = report.progress();
        let progress = match report.state() {
            Ok(ProgressState::Remove) => Progress::Cleared,
            Ok(ProgressState::Set) => Progress::Set(value.unwrap_or(0).min(100)),
            Ok(ProgressState::Error) => Progress::Error(value),
            Ok(ProgressState::Indeterminate) => Progress::Indeterminate,
            Ok(ProgressState::Pause) => Progress::Paused(value),
            _ => return,
        };
        s.push(Event::Progress(progress));
    })?;

    let s = signals;
    term.on_semantic_prompt(move |_, prompt| {
        let mark = match prompt.kind() {
            Ok(SemanticPromptKind::PromptStart) => PromptMark::PromptStart,
            Ok(SemanticPromptKind::InputStart) => PromptMark::InputStart,
            Ok(SemanticPromptKind::OutputStart) => {
                let command = prompt.command();
                PromptMark::OutputStart {
                    command: (!command.is_empty())
                        .then(|| String::from_utf8_lossy(command).into_owned()),
                }
            }
            Ok(SemanticPromptKind::CommandEnd) => PromptMark::CommandEnd {
                exit_code: prompt.exit_code(),
            },
            _ => return,
        };
        s.push(Event::Prompt(mark));
    })?;

    // Programs ask what they are talking to; without answers some wait for
    // a reply that never comes.
    term.on_xtversion(|_| Some(concat!("ljos-hof ", env!("CARGO_PKG_VERSION"))))?;
    term.on_device_attributes(|_| {
        Some(DeviceAttributes {
            primary: PrimaryDeviceAttributes::new(
                ConformanceLevel::VT220,
                &[
                    DeviceAttributeFeature::COLUMNS_132,
                    DeviceAttributeFeature::SELECTIVE_ERASE,
                    DeviceAttributeFeature::ANSI_COLOR,
                    DeviceAttributeFeature::CLIPBOARD,
                ],
            ),
            secondary: SecondaryDeviceAttributes {
                device_type: DeviceType::VT220,
                firmware_version: 1,
                rom_cartridge: 0,
            },
            tertiary: TertiaryDeviceAttributes::default(),
        })
    })?;
    term.on_size(|t| {
        let (cols, rows) = (t.cols().ok()?, t.rows().ok()?);
        let (w, h) = (t.width_px().ok()?, t.height_px().ok()?);
        Some(SizeReportSize {
            rows,
            columns: cols,
            cell_width: w / u32::from(cols.max(1)),
            cell_height: h / u32::from(rows.max(1)),
        })
    })?;
    let dark = identity.scheme;
    term.on_color_scheme(move |_| {
        Some(if dark.load(std::sync::atomic::Ordering::Relaxed) {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        })
    })?;
    Ok(())
}
