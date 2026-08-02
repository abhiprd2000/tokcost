//! Live command wrapping: run a child process, tee its stdout through to
//! our own stdout unchanged, and render a running token-count/cost ticker
//! on stderr as output arrives.
//!
//! Three known, disclosed limitations (see the README):
//!
//! - The child's stdout is a pipe, not a terminal. Many programs fully
//!   buffer their output rather than line-buffering it when they detect a
//!   non-TTY stdout, so the ticker can update in bursts instead of
//!   continuously for such programs. A pseudo-TTY would fix this, but
//!   `openpty` (Unix) and `ConPTY` (Windows) are unrelated, unsafe,
//!   platform-specific APIs — building and maintaining both without any
//!   crate is out of scope for what this tool is trying to be.
//! - The ticker itself is written to stderr (so stdout stays clean for
//!   piping), which means it can interleave with anything the child writes
//!   to its *own* stderr, since that's inherited directly.
//! - The ticker prints one line per update rather than redrawing a single
//!   line in place. Stdout is tee'd byte-for-byte (it has to stay a
//!   faithful copy of the child's real output for piping), so it can end
//!   mid-line at any point; a `\r`-based redraw sharing the same terminal
//!   would then land on whatever partial line the tee last wrote and
//!   corrupt it. Only ever appending a newline-terminated ticker frame
//!   avoids that collision at the cost of a scrolling log instead of a
//!   single animated line.

use crate::render;
use crate::{pricing, vocab};
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TICKER_INTERVAL: Duration = Duration::from_millis(100);
const READ_CHUNK_SIZE: usize = 8192;

pub struct MeterOptions<'a> {
    pub model: &'a str,
    pub command: &'a str,
    pub args: &'a [String],
}

/// Run `options.command`, tee its stdout to our own stdout, and print a
/// live token/cost ticker to stderr. Returns the child's exit code.
pub fn run(options: &MeterOptions) -> io::Result<i32> {
    run_with(options, &mut io::stdout(), &mut io::stderr())
}

fn run_with(
    options: &MeterOptions,
    out: &mut impl Write,
    ticker_out: &mut impl Write,
) -> io::Result<i32> {
    let mut child = Command::new(options.command)
        .args(options.args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut child_stdout = child.stdout.take().expect("stdout was piped");

    let mut collected = Vec::new();
    let mut buf = [0u8; READ_CHUNK_SIZE];
    let mut last_render = Instant::now()
        .checked_sub(TICKER_INTERVAL)
        .unwrap_or_else(Instant::now);

    loop {
        let n = child_stdout.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        out.flush()?;
        collected.extend_from_slice(&buf[..n]);

        if last_render.elapsed() >= TICKER_INTERVAL {
            render_ticker(ticker_out, &collected, options.model)?;
            last_render = Instant::now();
        }
    }

    render_ticker(ticker_out, &collected, options.model)?;

    let status = child.wait()?;
    Ok(status.code().unwrap_or(1))
}

/// The text streamed so far is being *generated* by the wrapped command
/// (e.g. an LLM CLI's response), so it's costed at the model's output
/// rate, not its input rate.
fn ticker_line(collected: &[u8], model: &str) -> String {
    let text = String::from_utf8_lossy(collected);
    let count = vocab::count(&text, model);
    let cost = pricing::estimate_cost(model, 0, count.value());

    let label = if count.is_estimated() {
        "~tokens"
    } else {
        "tokens"
    };
    let tokens_part = format!("{} {label}", render::format_int(count.value() as u64));
    let cost_part = match cost {
        Some(c) => render::format_usd(c.total_usd()),
        None => "n/a".to_string(),
    };

    format!(
        "{} \u{b7} {}",
        render::colorize(&tokens_part, render::CYAN),
        render::colorize(&cost_part, render::GREEN),
    )
}

fn render_ticker(out: &mut impl Write, collected: &[u8], model: &str) -> io::Result<()> {
    writeln!(out, "{}", ticker_line(collected, model))?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticker_line_reports_exact_count_for_bpe_models() {
        let line = ticker_line(b"hello world", "gpt-4o");
        assert!(line.contains("tokens"));
        assert!(!line.contains("~tokens"));
    }

    #[test]
    fn ticker_line_marks_claude_counts_as_estimated() {
        let line = ticker_line(b"hello world", "claude-sonnet-5");
        assert!(line.contains("~tokens"));
    }

    #[test]
    fn ticker_line_shows_na_for_unpriced_models() {
        let line = ticker_line(b"hello world", "some-fictional-model");
        assert!(line.contains("n/a"));
    }

    #[test]
    fn ticker_line_has_no_embedded_newlines() {
        // render_ticker adds exactly one trailing newline via `writeln!`;
        // the line itself must never contain one, or a redraw could still
        // land mid-line in whatever the child tee'd to stdout meanwhile.
        assert!(!ticker_line(b"x", "gpt-4o").contains('\n'));
    }

    #[cfg(unix)]
    #[test]
    fn run_with_tees_stdout_and_returns_exit_code() {
        let options = MeterOptions {
            model: "gpt-4o",
            command: "sh",
            args: &["-c".to_string(), "printf 'hello world'; exit 3".to_string()],
        };
        let mut out = Vec::new();
        let mut ticker = Vec::new();
        let code = run_with(&options, &mut out, &mut ticker).unwrap();

        assert_eq!(code, 3);
        assert_eq!(String::from_utf8(out).unwrap(), "hello world");
        // At least the final render happened, reporting real token counts.
        let ticker_text = String::from_utf8(ticker).unwrap();
        assert!(ticker_text.contains("tokens"));
    }

    #[cfg(windows)]
    #[test]
    fn run_with_tees_stdout_and_returns_exit_code() {
        let options = MeterOptions {
            model: "gpt-4o",
            command: "cmd",
            args: &[
                "/C".to_string(),
                "echo|set /p=hello world & exit 3".to_string(),
            ],
        };
        let mut out = Vec::new();
        let mut ticker = Vec::new();
        let code = run_with(&options, &mut out, &mut ticker).unwrap();

        assert_eq!(code, 3);
        assert_eq!(String::from_utf8(out).unwrap(), "hello world");
        let ticker_text = String::from_utf8(ticker).unwrap();
        assert!(ticker_text.contains("tokens"));
    }
}
