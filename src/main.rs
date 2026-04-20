mod app;
mod config;
mod git;
mod model;
mod paths;
mod state;
mod ui;

use std::io::{self, Stdout};
use std::panic;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::{AppMessage, AppState, Direction};
use crate::config::Config;
use crate::paths::AppPaths;

type Tui = Terminal<CrosstermBackend<Stdout>>;

#[derive(Parser)]
#[command(name = "grove", version, about)]
struct Cli {
    /// Override config file path
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Print resolved config, state and log paths, then exit
    #[arg(long)]
    print_paths: bool,

    /// Create an empty config file at the default location
    #[arg(long)]
    init: bool,
}

fn main() -> ExitCode {
    match run_cli() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_cli() -> Result<ExitCode> {
    let cli = Cli::parse();
    let paths = AppPaths::resolve()?;
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(|| paths.config_file.clone());

    if cli.print_paths {
        print_paths(&paths);
        return Ok(ExitCode::SUCCESS);
    }

    if cli.init {
        Config::write_template(&config_path)?;
        println!("Created config at {}", config_path.display());
        println!("Edit it to add your repositories, then run `grove`.");
        return Ok(ExitCode::SUCCESS);
    }

    if !config_path.exists() {
        print_first_run_message(&config_path);
        return Ok(ExitCode::SUCCESS);
    }

    let config = Config::load(&config_path)?;
    let mut app = AppState::load(&config)?;
    if let Some(persisted) = state::load(&paths.state_file)? {
        app.apply_persisted(persisted);
    }

    install_panic_hook();
    let mut tui = init_terminal()?;
    let result = run(&mut tui, &mut app);
    restore_terminal()?;
    result?;

    if let Err(err) = state::save(&app.to_persisted(), &paths.state_file) {
        eprintln!("warning: failed to save state: {err:#}");
    }
    Ok(ExitCode::SUCCESS)
}

fn run(tui: &mut Tui, app: &mut AppState) -> Result<()> {
    loop {
        tui.draw(|frame| ui::render(frame, app))?;

        if let Event::Key(key) = event::read()? {
            let msg = key_to_message(key, app.ui.modal.is_some());
            app.update(msg);
            if app.should_quit {
                break;
            }
        }
    }
    Ok(())
}

fn key_to_message(key: KeyEvent, modal_open: bool) -> AppMessage {
    if key.kind != KeyEventKind::Press {
        return AppMessage::NoOp;
    }
    if modal_open {
        return match key.code {
            KeyCode::Char('?') | KeyCode::Esc => AppMessage::CloseModal,
            _ => AppMessage::NoOp,
        };
    }
    match key.code {
        KeyCode::Char('q') => AppMessage::Quit,
        KeyCode::Char('?') => AppMessage::ToggleHelp,
        KeyCode::Char('j') | KeyCode::Down => AppMessage::MoveCursor(Direction::Down),
        KeyCode::Char('k') | KeyCode::Up => AppMessage::MoveCursor(Direction::Up),
        KeyCode::Char('h') | KeyCode::Left => AppMessage::CollapseOrAscend,
        KeyCode::Char('l') | KeyCode::Right => AppMessage::ExpandOrDescend,
        KeyCode::Enter => AppMessage::Activate,
        _ => AppMessage::NoOp,
    }
}

fn print_paths(paths: &AppPaths) {
    println!("config: {}", paths.config_file.display());
    println!("state:  {}", paths.state_file.display());
    println!("log:    {}", paths.log_file.display());
}

fn print_first_run_message(config_path: &std::path::Path) {
    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    println!();
    println!("No config file found. Create one at:");
    println!("  {}", config_path.display());
    println!();
    println!("Or run:");
    println!("  grove --init");
    println!();
    println!("to create an empty template.");
}

fn init_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(io::stdout()))?)
}

fn restore_terminal() -> Result<()> {
    execute!(io::stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

fn install_panic_hook() {
    let original = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original(info);
    }));
}
