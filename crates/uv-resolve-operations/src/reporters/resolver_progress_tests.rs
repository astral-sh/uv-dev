use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Result;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle, TermLike};
use uv_command_support::Printer;
use uv_distribution_types::VersionOrUrlRef;
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pep508::VerbatimUrl;
use uv_resolver::ResolverReporter as _;

use super::ResolverReporter;

const WIDTH: u16 = 160;
const INITIAL_MESSAGE: &str = "Resolving dependencies...";

#[derive(Clone, Debug, Default)]
struct RecordingTerm(Arc<Mutex<Vec<String>>>);

impl RecordingTerm {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl TermLike for RecordingTerm {
    fn width(&self) -> u16 {
        WIDTH
    }

    fn move_cursor_up(&self, _count: usize) -> io::Result<()> {
        Ok(())
    }

    fn move_cursor_down(&self, _count: usize) -> io::Result<()> {
        Ok(())
    }

    fn move_cursor_right(&self, _count: usize) -> io::Result<()> {
        Ok(())
    }

    fn move_cursor_left(&self, _count: usize) -> io::Result<()> {
        Ok(())
    }

    fn write_line(&self, value: &str) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{value}\n"));
        Ok(())
    }

    fn write_str(&self, value: &str) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value.to_owned());
        Ok(())
    }

    fn clear_line(&self) -> io::Result<()> {
        Ok(())
    }

    fn flush(&self) -> io::Result<()> {
        Ok(())
    }
}

fn reporter(target: ProgressDrawTarget) -> Result<ResolverReporter> {
    let root = ProgressBar::with_draw_target(None, target);
    root.set_style(ProgressStyle::with_template("{msg}")?);
    root.set_message(INITIAL_MESSAGE);
    let mut reporter = ResolverReporter::from(Printer::NoProgress);
    reporter.reporter.root.disable_steady_tick();
    reporter.reporter.printer = Printer::Default;
    reporter.reporter.root = root;
    Ok(reporter)
}

fn inputs() -> Result<(PackageName, Version, VerbatimUrl)> {
    Ok((
        "authored-package".parse()?,
        "1.2.3rc4".parse()?,
        VerbatimUrl::parse_url("https://example.invalid/authored-1.2.3.whl#sha256=abc")?,
    ))
}

#[test]
fn hidden_resolver_progress_keeps_its_message() -> Result<()> {
    let (name, version, url) = inputs()?;
    let reporter = reporter(ProgressDrawTarget::hidden())?;
    assert!(reporter.reporter.root.is_hidden());
    reporter.on_progress(&name, &VersionOrUrlRef::Version(&version));
    assert_eq!(reporter.reporter.root.message(), INITIAL_MESSAGE);
    reporter.on_progress(&name, &VersionOrUrlRef::Url(&url));
    assert_eq!(reporter.reporter.root.message(), INITIAL_MESSAGE);
    reporter.on_complete();
    assert_eq!(reporter.reporter.root.message(), "");
    assert!(reporter.reporter.root.is_finished());
    Ok(())
}

#[test]
fn visible_resolver_progress_preserves_output() -> Result<()> {
    let (name, version, url) = inputs()?;
    let terminal = RecordingTerm::default();
    let reporter = reporter(ProgressDrawTarget::term_like(Box::new(terminal.clone())))?;
    assert!(!reporter.reporter.root.is_hidden());
    terminal.take();

    reporter.on_progress(&name, &VersionOrUrlRef::Version(&version));
    let message = "authored-package==1.2.3rc4";
    assert_eq!(reporter.reporter.root.message(), message);
    assert_eq!(
        terminal.take(),
        [
            message.to_owned(),
            " ".repeat(usize::from(WIDTH) - message.len())
        ]
    );

    reporter.on_progress(&name, &VersionOrUrlRef::Url(&url));
    let message = "authored-package @ https://example.invalid/authored-1.2.3.whl#sha256=abc";
    assert_eq!(reporter.reporter.root.message(), message);
    assert_eq!(
        terminal.take(),
        [
            message.to_owned(),
            " ".repeat(usize::from(WIDTH) - message.len())
        ]
    );

    reporter.on_complete();
    assert_eq!(reporter.reporter.root.message(), "");
    assert!(reporter.reporter.root.is_finished());
    assert!(terminal.take().is_empty());
    Ok(())
}
