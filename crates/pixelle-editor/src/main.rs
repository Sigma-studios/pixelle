//! The Pixelle editor.
//!
//! ```text
//! pixelle-editor [project folder] [--open <file>] [--anim <name>] [--zoom <n>] [--tool <name>] [--layer <name>]
//!                [--screenshot <out.png>]
//! ```
//!
//! With no folder, it opens the project opened last. `--open` opens one of its files in a tab, by
//! name (`weapons`, `fx/bullet`), and `--anim` one of its animations in the animation view. `--screenshot` renders the window once everything has loaded,
//! saves it and quits: a way to look at the editor from a script.

mod anim;
mod app;
mod backend;
mod canvas;
mod colour;
mod doc;
mod panels;
mod tools;
mod view;
mod workspace;

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut args = app::Args { project: None, remember: true, screenshot: None, open: None, anim: None, zoom: None, tool: None, layer: None };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or_else(|| anyhow::anyhow!("{a} needs a value"));
        match a.as_str() {
            "--screenshot" => args.screenshot = Some(PathBuf::from(value()?)),
            "--open" => args.open = Some(value()?),
            "--anim" => args.anim = Some(value()?),
            "--zoom" => args.zoom = Some(value()?.parse()?),
            "--tool" => args.tool = Some(value()?),
            "--layer" => args.layer = Some(value()?),
            "-h" | "--help" => {
                println!("pixelle-editor [project folder] [--open <file>] [--anim <name>] [--zoom <n>] [--tool <name>] [--layer <name>] [--screenshot <out.png>]");
                return Ok(());
            }
            _ if a.starts_with("--") => anyhow::bail!("unknown option {a}"),
            _ => args.project = Some(PathBuf::from(a)),
        }
    }
    let editor = app::Editor::new(args)?;
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_title("Pixelle"),
        ..Default::default()
    };
    eframe::run_native("Pixelle", options, Box::new(|_| Ok(Box::new(editor))))
        .map_err(|e| anyhow::anyhow!("{e}"))
}
