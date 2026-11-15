//! `cargo run -p icon-index` writes `crates/ui/ui/icons.slint` and `assets/icons/overview.html`;
//! with `--check` it only checks (exit code 1 on errors).

use icon_index::{Layout, check_icons, load_icons, overview_html, repository_root, slint_index};
use std::process::ExitCode;

fn main() -> ExitCode {
    let check_only = std::env::args().any(|argument| argument == "--check");
    let layout = Layout::of_repository(&repository_root());
    let icons = match load_icons(&layout.icons) {
        Ok(icons) => icons,
        Err(error) => {
            eprintln!("cannot read {}: {error}", layout.icons.display());
            return ExitCode::FAILURE;
        }
    };
    let mut problems = check_icons(&icons);
    let index = slint_index(&icons);

    if check_only {
        let current = std::fs::read_to_string(&layout.slint_index).unwrap_or_default().replace("\r\n", "\n");
        if current != index {
            problems.push("crates/ui/ui/icons.slint is outdated (run: cargo run -p icon-index)".to_owned());
        }
    } else if let Err(error) = std::fs::write(&layout.slint_index, &index)
        .and_then(|()| std::fs::write(&layout.overview, overview_html(&icons)))
    {
        eprintln!("cannot write icon index: {error}");
        return ExitCode::FAILURE;
    }

    for problem in &problems {
        eprintln!("ERROR {problem}");
    }
    println!("{} icons checked, {} problems.", icons.len(), problems.len());
    if problems.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
