//! The desktop app's window without a display: drawn offscreen in a scene, driven by a script,
//! and saved as pictures, for looking at the UI and at what a change does to it.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use bukospeak_desktop::harness::{Harness, Scene, Step, scenes};
use clap::{Args, Parser, Subcommand, ValueEnum};
use gpui::{px, size};
use image::{GenericImage, Rgba, RgbaImage};

#[derive(Parser)]
#[command(about = "The desktop app's window, drawn offscreen in a scene and driven by a script")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the scenes.
    List,
    /// Draw a scene, after the script's steps, into a PNG at twice the window's size.
    Snap {
        #[command(flatten)]
        view: View,
        #[arg(long, short, default_value = "snap.png")]
        out: PathBuf,
    },
    /// Draw frames of what the script's last step sets moving, from the step on, side by side in
    /// one PNG.
    Film {
        #[command(flatten)]
        view: View,
        /// How many frames.
        #[arg(long, default_value_t = 8)]
        frames: u32,
        /// How far apart, in milliseconds.
        #[arg(long, default_value_t = 50)]
        every: u64,
        #[arg(long, short, default_value = "film.png")]
        out: PathBuf,
    },
    /// Print the named elements drawn, and where: what a script clicks, hovers and checks.
    Marks {
        #[command(flatten)]
        view: View,
    },
    /// Draw every scene, light and dark, into a folder with a page that shows them all.
    Gallery {
        #[arg(default_value = "gallery")]
        folder: PathBuf,
        /// The window's size, in logical pixels.
        #[arg(long, default_value = "380x560", value_parser = parse_size)]
        size: (f32, f32),
    },
}

#[derive(Args)]
struct View {
    /// The scene, as `list` names it.
    scene: String,
    /// What to do to the window before: `click settings; type Kiwi; key enter; wait 200`.
    #[arg(long, default_value = "")]
    steps: String,
    #[arg(long, value_enum, default_value_t = Theme::Dark)]
    theme: Theme,
    /// The window's size, in logical pixels.
    #[arg(long, default_value = "380x560", value_parser = parse_size)]
    size: (f32, f32),
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Theme {
    Light,
    Dark,
}

fn parse_size(text: &str) -> Result<(f32, f32), String> {
    let (width, height) = text.split_once('x').ok_or("WIDTHxHEIGHT")?;
    Ok((width.parse().map_err(|_| "a width")?, height.parse().map_err(|_| "a height")?))
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::List => {
            for scene in scenes() {
                println!("{:<22} {}", scene.name, scene.about);
            }
        }
        Command::Snap { view, out } => {
            let mut harness = open(&view.scene, &Step::parse(&view.steps)?, &view)?;
            save(&harness.snap()?, &out)?;
        }
        Command::Film { view, frames, every, out } => {
            let mut steps = Step::parse(&view.steps)?;
            let last = steps.pop().context("a film films the script's last step; there is none")?;
            let mut harness = open(&view.scene, &steps, &view)?;
            let shots = harness.film(&last, frames, Duration::from_millis(every))?;
            save(&side_by_side(&shots)?, &out)?;
        }
        Command::Marks { view } => {
            let mut harness = open(&view.scene, &Step::parse(&view.steps)?, &view)?;
            for (name, bounds) in harness.marks()? {
                let (origin, size) = (bounds.origin, bounds.size);
                println!(
                    "{name:<28} x {:>5.1}  y {:>5.1}  w {:>5.1}  h {:>5.1}",
                    f32::from(origin.x),
                    f32::from(origin.y),
                    f32::from(size.width),
                    f32::from(size.height)
                );
            }
        }
        Command::Gallery { folder, size } => gallery(&folder, size)?,
    }
    Ok(())
}

fn scene(name: &str) -> anyhow::Result<Scene> {
    let all = scenes();
    let names: Vec<&str> = all.iter().map(|scene| scene.name).collect();
    let names = names.join(", ");
    match all.into_iter().find(|scene| scene.name == name) {
        Some(scene) => Ok(scene),
        None => bail!("no scene is called {name}; there are: {names}"),
    }
}

/// The window on the scene `name`, as `view` has it, after `steps`.
#[allow(clippy::disallowed_methods, reason = "the window is as large as the command line asks")]
fn open(name: &str, steps: &[Step], view: &View) -> anyhow::Result<Harness> {
    let scene = scene(name)?;
    let mut harness = Harness::open(&scene, size(px(view.size.0), px(view.size.1)), view.theme == Theme::Dark)?;
    for step in steps {
        harness.step(step)?;
    }
    Ok(harness)
}

fn save(image: &RgbaImage, path: &Path) -> anyhow::Result<()> {
    if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        fs::create_dir_all(folder)?;
    }
    image.save(path).with_context(|| format!("writing {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

/// The frames in a row, a gap between each.
fn side_by_side(frames: &[RgbaImage]) -> anyhow::Result<RgbaImage> {
    const GAP: u32 = 16;
    let first = frames.first().context("no frames")?;
    let (width, height) = first.dimensions();
    let count = frames.len() as u32;
    let mut sheet = RgbaImage::from_pixel(count * width + (count - 1) * GAP, height, Rgba([128, 128, 128, 255]));
    for (index, frame) in frames.iter().enumerate() {
        sheet.copy_from(frame, index as u32 * (width + GAP), 0)?;
    }
    Ok(sheet)
}

#[allow(clippy::disallowed_methods, reason = "the window is as large as the command line asks")]
fn gallery(folder: &Path, (width, height): (f32, f32)) -> anyhow::Result<()> {
    fs::create_dir_all(folder)?;
    let mut rows = String::new();
    for scene in scenes() {
        let mut cells = String::new();
        for (theme, dark) in [("light", false), ("dark", true)] {
            let file = format!("{}-{theme}.png", scene.name);
            let mut harness = Harness::open(&scene, size(px(width), px(height)), dark)?;
            save(&harness.snap()?, &folder.join(&file))?;
            write!(cells, r#"<img src="{file}" width="{width}" alt="{} ({theme})">"#, scene.name)?;
        }
        write!(rows, r#"<section><h2>{}</h2><p>{}</p><div>{cells}</div></section>"#, scene.name, scene.about)?;
    }
    let page = format!(
        r#"<!doctype html><meta charset="utf-8"><title>BukoSpeak scenes</title>
<style>body{{font:14px system-ui;background:#808080;margin:24px}}section{{display:inline-block;vertical-align:top;margin:0 24px 32px 0}}
h2{{font-size:15px;margin:0}}p{{margin:2px 0 8px;color:#222}}div{{display:flex;gap:12px}}img{{border-radius:12px}}</style>
{rows}"#
    );
    fs::write(folder.join("index.html"), page)?;
    println!("{}", folder.join("index.html").display());
    Ok(())
}
