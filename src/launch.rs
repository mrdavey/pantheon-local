//! The foreground launcher displays short, authenticated terminal hyperlinks.
//! The terminal stays in cooked mode so the shell's signal keys still work.
mod mark;

use nix::{
    poll::{PollFd, PollFlags, PollTimeout, poll},
    unistd::getpgrp,
};
use rustix::termios::{
    LocalModes, OptionalActions, Termios, tcgetattr, tcgetpgrp, tcgetwinsize, tcsetattr, ttyname,
};
use std::{
    env,
    fmt::Write as _,
    future::Future,
    io::{self, IsTerminal, Write},
    os::fd::AsFd,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedReceiver;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const OPEN_LINK: &str = "Open the Pantheon Local link above in your browser.";

pub fn interactive() -> bool {
    io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && env::var_os("TERM").is_some_and(|term| term != "dumb" && !term.is_empty())
        && env::var("SWARM_PLAIN").as_deref() != Ok("1")
        && foreground()
}

fn foreground() -> bool {
    tcgetpgrp(io::stdin())
        .is_ok_and(|foreground| foreground.as_raw_nonzero().get() == getpgrp().as_raw())
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Size {
    columns: usize,
    rows: usize,
}

impl Size {
    fn current() -> Option<Self> {
        let size = tcgetwinsize(io::stdout()).ok()?;
        (size.ws_col > 0 && size.ws_row > 0).then_some(Self {
            columns: usize::from(size.ws_col),
            rows: usize::from(size.ws_row),
        })
    }
}

#[derive(PartialEq, Eq)]
struct Line {
    row: usize,
    column: usize,
    style: &'static str,
    text: String,
    href: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Serve,
    Owner,
    Attached,
}

pub enum Notice {
    Update(serde_json::Value),
    Message(String),
}

#[derive(Default)]
struct Notices {
    update: Option<serde_json::Value>,
    message: Option<String>,
    browser_failed: bool,
}

fn short_link(link: &str) -> &str {
    link.split_once("/#").map_or(link, |(origin, _)| origin)
}

fn hyperlink(label: &str, target: &str) -> String {
    format!("\x1b]8;;{target}\x1b\\{label}\x1b]8;;\x1b\\")
}

fn clipped(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .filter(|c| !c.is_control())
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= width
        })
        .collect()
}

fn frame(size: Size, link: &str, phase: f32, notices: &Notices, mode: Mode) -> Vec<Line> {
    let Size { columns, rows } = size;
    let mut lines = Vec::new();
    let mut place = |row, text: &str, style, centered, href: Option<&str>| {
        let text = clipped(text, columns);
        let length = text.width();
        let column = if centered {
            (columns - length) / 2 + 1
        } else {
            2
        };
        lines.push(Line {
            row,
            column,
            style,
            text,
            href: href.map(str::to_owned),
        });
    };
    let update = notices.update.as_ref().and_then(|v| {
        Some((
            format!("Update available · v{}", v["latest"].as_str()?),
            v["url"].as_str()?,
        ))
    });
    let message = if notices.browser_failed {
        Some("Browser did not open. Use the link above.")
    } else {
        notices.message.as_deref()
    };
    let reminder = match mode {
        Mode::Attached => "Keep the service window open.",
        _ if columns >= 49 => "Keep this window open while using Pantheon Local.",
        _ => "Keep this window open.",
    };
    if columns < 28 || rows < 7 {
        place(1, short_link(link), "", true, Some(link));
        if rows >= 3
            && let Some((label, url)) = update
        {
            place(2, &label, "\x1b[1;33m", true, Some(url));
        }
        if rows >= 4
            && let Some(text) = message
        {
            place(3, text, "", true, None);
        }
        if rows >= 2 {
            place(rows, reminder, "", true, None);
        }
        return lines;
    }
    let notice_rows = usize::from(update.is_some()) + usize::from(message.is_some());
    let header = rows >= 10;
    let top = if header { 3 } else { 1 };
    let footer = rows - 1;
    let available = footer - top;
    let height = if columns >= 46 && available >= 19 + notice_rows {
        14
    } else if available >= 12 + notice_rows {
        8
    } else {
        0
    };
    let width = if height == 14 { 40 } else { 24 };
    let block = height + usize::from(height > 0) + 2 + notice_rows;
    let mut row = top + available.saturating_sub(block) / 2;
    if header {
        place(2, "Pantheon Local", "\x1b[1m", false, None);
    }
    if height > 0 {
        for art in mark::portico(width, height, phase) {
            place(row, &art, "\x1b[38;5;111m", true, None);
            row += 1;
        }
        row += 1;
    }
    place(row, "●  Workbench online", "\x1b[1m", true, None);
    row += 1;
    place(row, short_link(link), "\x1b[4;36m", true, Some(link));
    if let Some((label, url)) = update {
        row += 1;
        place(row, &label, "\x1b[1;33m", true, Some(url));
    }
    if let Some(text) = message {
        place(row + 1, text, "\x1b[33m", true, None);
    }
    place(footer, reminder, "", true, None);
    let hints = match (columns >= 43, mode) {
        (true, Mode::Attached) => ("  Open browser     ", "  Close view"),
        (true, _) => ("  Open browser     ", "  Stop service"),
        (false, Mode::Attached) => ("  Open    ", "  Close"),
        (false, _) => ("  Open    ", "  Stop"),
    };
    let keys = [
        ("Enter", "\x1b[1;36m"),
        (hints.0, ""),
        ("Ctrl+C", "\x1b[1;36m"),
        (hints.1, ""),
    ];
    let width: usize = keys.iter().map(|(text, _)| text.len()).sum();
    let mut column = (columns - width) / 2 + 1;
    for (text, style) in keys {
        lines.push(Line {
            row: rows,
            column,
            style,
            text: text.into(),
            href: None,
        });
        column += text.len();
    }
    if header {
        let version = concat!("v", env!("CARGO_PKG_VERSION"));
        lines.push(Line {
            row: 2,
            column: columns - version.len(),
            style: "",
            text: version.into(),
            href: None,
        });
    }
    lines
}

struct Screen {
    output: std::fs::File,
    original_input: Termios,
    pending: Vec<u8>,
    size: Option<Size>,
    previous: Vec<Line>,
    color: bool,
    start: Instant,
    motion: bool,
}
impl Screen {
    fn enter() -> Option<Self> {
        if !interactive() || Size::current().is_none() {
            return None;
        }
        // A separate file description leaves stdout's flags alone. Terminal
        // backpressure must never stall HTTP requests or accumulate frames.
        use rustix::fs::{Mode, OFlags, open};
        let path = ttyname(io::stdout(), Vec::new()).ok()?;
        let output = open(
            path.as_c_str(),
            OFlags::WRONLY | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .ok()?;
        let original_input = tcgetattr(io::stdin()).ok()?;
        let mut quiet = original_input.clone();
        quiet
            .local_modes
            .remove(LocalModes::ECHO | LocalModes::ECHONL);
        tcsetattr(io::stdin(), OptionalActions::Now, &quiet).ok()?;
        let mut screen = Self {
            original_input,
            output: output.into(),
            pending: Vec::new(),
            size: None,
            previous: Vec::new(),
            color: env::var_os("NO_COLOR").is_none(),
            start: Instant::now(),
            motion: env::var("REDUCE_MOTION").as_deref() != Ok("1"),
        };
        screen.output.write_all(b"\x1b[?1049h\x1b[?25l").ok()?;
        Some(screen)
    }

    fn flush_frame(&mut self) -> io::Result<()> {
        match self.output.write(&self.pending) {
            Ok(count) => {
                self.pending.drain(..count);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
        Ok(())
    }

    fn open_requested(&self) -> bool {
        let stdin = io::stdin();
        let mut input = [PollFd::new(stdin.as_fd(), PollFlags::POLLIN)];
        if !poll(&mut input, PollTimeout::ZERO).is_ok_and(|n| n > 0)
            || !input[0]
                .revents()
                .is_some_and(|flags| flags.contains(PollFlags::POLLIN))
        {
            return false;
        }
        let mut bytes = [0; 256];
        rustix::io::read(&stdin, &mut bytes).is_ok_and(|n| bytes[..n].contains(&b'\n'))
    }

    fn refresh(&mut self, link: &str, notices: &Notices, mode: Mode) -> io::Result<()> {
        let Some(size) = Size::current() else {
            return Ok(());
        };
        if !self.pending.is_empty() {
            self.flush_frame()?;
            if !self.pending.is_empty() {
                return Ok(());
            }
        }
        let resized = self.size != Some(size);
        if !resized && !self.motion {
            return Ok(());
        }
        let phase = if self.motion {
            self.start.elapsed().as_secs_f32() * 0.76
        } else {
            0.0
        };
        let next = frame(size, link, phase, notices, mode);
        let mut draw = String::new();
        if resized {
            draw.push_str("\x1b[2J");
        }
        for (i, line) in next.iter().enumerate() {
            if resized || self.previous.get(i) != Some(line) {
                let style = if self.color { line.style } else { "" };
                write!(
                    draw,
                    "\x1b[{};{}H{style}{}\x1b[0m",
                    line.row,
                    line.column,
                    line.href
                        .as_deref()
                        .map_or_else(|| line.text.clone(), |url| hyperlink(&line.text, url))
                )
                .unwrap();
            }
        }
        if !draw.is_empty() {
            self.pending = draw.into_bytes();
            self.flush_frame()?;
        }
        self.size = Some(size);
        self.previous = next;
        Ok(())
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = tcsetattr(io::stdin(), OptionalActions::Now, &self.original_input);
        // Cancel any partial escape sequence before returning to scrollback.
        let _ = self
            .output
            .write_all(b"\x18\x1b]8;;\x1b\\\x1b[0m\x1b[?25h\x1b[?1049l");
    }
}

pub async fn monitor<T>(
    server: impl Future<Output = T>,
    mode: Mode,
    link: &str,
    mut notices: UnboundedReceiver<Notice>,
) -> T {
    let browser = mode != Mode::Serve;
    let mut screen = if browser { Screen::enter() } else { None };
    let interactive_display = screen.is_some();
    if !interactive_display {
        if mode == Mode::Owner {
            println!("Keep this Terminal window open while you use Pantheon Local.");
        }
        println!("{link}");
    }
    let mut notices_on_screen = Notices::default();
    let launch_browser = || {
        let target = link.to_owned();
        tokio::task::spawn_blocking(move || crate::cli::open_browser(&target))
    };
    let mut opening = browser.then(launch_browser);
    let mut suspend = if screen.is_some() {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::from_raw(
            nix::libc::SIGTSTP,
        ))
        .ok()
    } else {
        None
    };
    let mut tick = tokio::time::interval(Duration::from_millis(110));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tokio::pin!(server);
    loop {
        tokio::select! {
            result = &mut server => return result,
            result = async { opening.as_mut().unwrap().await }, if opening.is_some() => {
                opening = None;
                notices_on_screen.browser_failed = !matches!(result, Ok(Ok(())));
                if let Some(active) = &mut screen { active.size = None; }
                else if !interactive_display && notices_on_screen.browser_failed { eprintln!("{OPEN_LINK}"); }
            }
            Some(notice) = notices.recv() => {
                match notice {
                    Notice::Update(update) => {
                        if !interactive_display && let Some(msg) = crate::update::cli_message(&update) { eprintln!("{msg}"); }
                        notices_on_screen.update = Some(update);
                    }
                    Notice::Message(msg) => {
                        if !interactive_display { eprintln!("{msg}"); }
                        notices_on_screen.message = Some(msg);
                    }
                }
                if let Some(active) = &mut screen { active.size = None; }
            }
            _ = async { suspend.as_mut().unwrap().recv().await }, if suspend.is_some() => {
                drop(screen.take());
                let _ = nix::sys::signal::raise(nix::sys::signal::Signal::SIGSTOP);
            }
            _ = tick.tick(), if interactive_display => {
                if !foreground() { continue; }
                if screen.is_none() { screen = Screen::enter(); }
                if let Some(active) = &mut screen {
                    if active.open_requested() && opening.is_none() { opening = Some(launch_browser()); }
                    if active.refresh(link, &notices_on_screen, mode).is_err() { active.size = None; }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_layout_fits_without_wrapping_or_covering_controls() {
        for columns in [28, 36, 45, 46, 50, 51, 80, 164] {
            for rows in [7, 12, 15, 16, 24, 26, 27, 54] {
                let lines = frame(
                    Size { columns, rows },
                    "http://127.0.0.1:65535/#example",
                    1.5,
                    &Notices {
                        update: Some(
                            serde_json::json!({"latest":"9.1.0","url":"https://example.com"}),
                        ),
                        message: Some("Profile unavailable".into()),
                        browser_failed: false,
                    },
                    Mode::Owner,
                );
                let mut occupied = std::collections::HashSet::new();
                for line in &lines {
                    assert!((1..=rows).contains(&line.row));
                    for col in line.column..line.column + line.text.chars().count() {
                        assert!((1..=columns).contains(&col));
                        assert!(
                            occupied.insert((line.row, col)),
                            "overlap at {columns}×{rows}"
                        );
                    }
                }
                assert!(
                    lines
                        .iter()
                        .any(|line| line.text.contains("Workbench online"))
                );
                assert!(lines.iter().any(|line| line.text.contains("Ctrl+C")));
            }
        }
    }

    #[test]
    fn rotating_mark_keeps_its_footprint_and_leaves_labels_still() {
        let size = Size {
            columns: 80,
            rows: 30,
        };
        let start = frame(size, "localhost", 0.0, &Notices::default(), Mode::Owner);
        let turned = frame(size, "localhost", 1.5, &Notices::default(), Mode::Owner);
        assert_eq!(start.len(), turned.len());
        assert!(start != turned);
        for (a, b) in start.iter().zip(turned) {
            assert_eq!((a.row, a.column), (b.row, b.column));
            if a.text != b.text {
                assert!(
                    a.text
                        .chars()
                        .all(|c| c == ' ' || ('\u{2800}'..='\u{28ff}').contains(&c))
                );
            }
        }
    }
}
