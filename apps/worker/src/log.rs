//! `tracing` on Workers: the application's `warn!` and `error!` lines reach
//! the console, which is what `wrangler tail` and Workers Logs show. Without
//! a subscriber every `tracing::error!` behind an internal error, and every
//! housekeeping step that failed, is a no-op in production.
//!
//! Installed once per isolate ([`install`]), writing each event as one
//! console line at its level. No timestamps (`SystemTime` panics on
//! `wasm32-unknown-unknown`; the console adds its own), no ANSI, `warn` and
//! up only (a fixed filter: `EnvFilter` would pull a regex engine into the
//! module), and each line once even while Restate replays a journal
//! ([`ReplayAwareFilter`], as the native engine filters). The lines carry
//! raffle IDs and error text, never personal data.

use std::io;

use restate_sdk::filter::ReplayAwareFilter;
use tracing::{Level, Metadata};
use tracing_subscriber::{
    Layer, filter::LevelFilter, fmt::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt,
};

/// Installs the subscriber, once; a second call (another request in the same
/// isolate, or a test) is a no-op.
pub fn install() {
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(Console)
                .with_ansi(false)
                .without_time()
                .with_target(false)
                .with_filter(LevelFilter::WARN)
                .with_filter(ReplayAwareFilter),
        )
        .try_init();
}

/// Makes a [`Line`] per event, at the event's level.
struct Console;

impl<'a> MakeWriter<'a> for Console {
    type Writer = Line;

    fn make_writer(&'a self) -> Line {
        Line::new(Level::INFO)
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Line {
        Line::new(*meta.level())
    }
}

/// One console line: collects what the formatter writes for an event and
/// prints it when dropped.
struct Line {
    level: Level,
    buf: Vec<u8>,
}

impl Line {
    fn new(level: Level) -> Self {
        Self {
            level,
            buf: Vec::new(),
        }
    }
}

impl io::Write for Line {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.buf);
        let text = text.trim_end();
        if text.is_empty() {
            return;
        }
        match self.level {
            Level::ERROR => worker::console_error!("{text}"),
            Level::WARN => worker::console_warn!("{text}"),
            _ => worker::console_log!("{text}"),
        }
    }
}
