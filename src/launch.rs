//! A foreground launch screen. Enter reveals a copyable link in the terminal.
//! The terminal stays in cooked mode so the shell's signal keys still work.
mod mark;

use nix::{
    poll::{PollFd, PollFlags, PollTimeout, poll},
    unistd::getpgrp,
};
use rustix::termios::{tcgetpgrp, tcgetwinsize, ttyname};
use std::{
    env,
    fmt::Write as _,
    future::Future,
    io::{self, IsTerminal, Write},
    os::fd::AsFd,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedReceiver;

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
        (size.ws_col >= 28 && size.ws_row >= 7).then_some(Self {
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
}

fn frame(size: Size, host: &str, phase: f32) -> Vec<Line> {
    let Size { columns, rows } = size;
    let mut lines = Vec::new();
    let mut place = |row, text: &str, style, centered| {
        let length = text.chars().count();
        let column = if centered {
            (columns - length) / 2 + 1
        } else {
            2
        };
        lines.push(Line {
            row,
            column,
            style,
            text: text.to_owned(),
        });
    };
    place(2, "Pantheon Local", "\x1b[1m", false);
    let version = concat!("v", env!("CARGO_PKG_VERSION"));
    let height = if rows >= 27 && columns >= 46 {
        14
    } else if rows >= 16 {
        8
    } else {
        0
    };
    let width = if height == 14 { 40 } else { 24 };
    let start = (rows - height - 3) / 2 + 1;
    if height > 0 {
        for (i, line) in mark::portico(width, height, phase).into_iter().enumerate() {
            place(start + i, &line, "\x1b[38;5;111m", true);
        }
    }
    let status_row = start + height + usize::from(height > 0);
    place(status_row, "●  Workbench online", "\x1b[1m", true);
    place(status_row + 1, host, "", true);
    let keys = if columns >= 51 {
        "Enter  Show browser link     Ctrl+C  Stop service"
    } else {
        "Enter  Link    Ctrl+C  Stop"
    };
    place(rows - 1, keys, "", true);
    lines.push(Line {
        row: 2,
        column: columns - version.len(),
        style: "",
        text: version.into(),
    });
    lines
}

struct Screen {
    output: std::fs::File,
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
        let mut screen = Self {
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

    fn refresh(&mut self, host: &str) -> io::Result<bool> {
        let Some(size) = Size::current() else {
            return Ok(false);
        };
        if !foreground() {
            return Ok(false);
        }
        let stdin = io::stdin();
        let mut input = [PollFd::new(stdin.as_fd(), PollFlags::POLLIN)];
        if poll(&mut input, PollTimeout::ZERO)? > 0 {
            if input[0]
                .revents()
                .is_some_and(|flags| flags.contains(PollFlags::POLLIN))
            {
                let _ = rustix::io::read(&stdin, &mut [0u8; 256]);
            }
            return Ok(false);
        }
        if !self.pending.is_empty() {
            self.flush_frame()?;
            if !self.pending.is_empty() {
                return Ok(true);
            }
        }
        let resized = self.size != Some(size);
        if !resized && !self.motion {
            return Ok(true);
        }
        let phase = if self.motion {
            self.start.elapsed().as_secs_f32() * 0.76
        } else {
            0.0
        };
        let next = frame(size, host, phase);
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
                    line.row, line.column, line.text
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
        Ok(true)
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        // Cancel any partial escape sequence before returning to scrollback.
        let _ = self.output.write_all(b"\x18\x1b[0m\x1b[?25h\x1b[?1049l");
    }
}

pub async fn monitor<T>(
    server: impl Future<Output = T>,
    browser: bool,
    link: &str,
    mut notices: UnboundedReceiver<String>,
) -> T {
    let mut screen = if browser { Screen::enter() } else { None };
    let plain = || {
        if browser {
            println!("Keep this Terminal window open while you use Pantheon Local.");
        }
        println!("{link}");
    };
    if screen.is_none() {
        plain();
    }
    let host = link
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("");
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
            Some(notice) = notices.recv() => {
                if screen.take().is_some() { plain(); }
                eprintln!("{notice}");
            }
            _ = async { suspend.as_mut().unwrap().recv().await }, if suspend.is_some() => {
                // Give the shell its screen and cursor before yielding the foreground.
                if screen.take().is_some() { plain(); }
                let _ = nix::sys::signal::raise(nix::sys::signal::Signal::SIGSTOP);
            }
            _ = tick.tick(), if screen.is_some() => {
                if let Some(active) = &mut screen && !active.refresh(host).unwrap_or(false) {
                    screen.take();
                    plain();
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
                let lines = frame(Size { columns, rows }, "127.0.0.1:65535", 1.5);
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
        let start = frame(size, "localhost", 0.0);
        let turned = frame(size, "localhost", 1.5);
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
