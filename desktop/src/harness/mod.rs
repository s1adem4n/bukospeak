mod host;
mod scenes;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use gpui::{
    AnyWindowHandle, AppContext, Bounds, HeadlessAppContext, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, OperatingSystem, Pixels, PlatformInput, Point, SharedString, Size,
};
use image::RgbaImage;

pub use scenes::{Scene, all as scenes};

use crate::call::Call;
use crate::ui::marks::Marks;
use crate::ui::{self, Client, Main, theme};

// Longer than any delay of the UI.
const SETTLE: Duration = Duration::from_millis(600);

fn keystroke(keys: &str) -> anyhow::Result<Keystroke> {
    Keystroke::parse_with_os(keys, OperatingSystem::Linux).map_err(|error| anyhow::anyhow!("{keys}: {error}"))
}

pub struct Harness {
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    pointer: Point<Pixels>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Click(String),
    Hover(String),
    Type(String),
    Key(String),
    Wait(Duration),
}

impl Step {
    pub fn parse(script: &str) -> anyhow::Result<Vec<Self>> {
        script
            .split(['\n', ';'])
            .map(str::trim)
            .filter(|step| !step.is_empty() && !step.starts_with('#'))
            .map(|step| {
                let (verb, rest) = step.split_once(' ').map_or((step, ""), |(verb, rest)| (verb, rest.trim()));
                Ok(match verb {
                    "click" => Self::Click(rest.to_owned()),
                    "hover" => Self::Hover(rest.to_owned()),
                    "type" => Self::Type(rest.to_owned()),
                    "key" => Self::Key(rest.to_owned()),
                    "wait" => Self::Wait(Duration::from_millis(rest.parse().with_context(|| format!("{step}: milliseconds"))?)),
                    _ => bail!("{step}: no such step"),
                })
            })
            .collect()
    }
}

impl Harness {
    pub fn open(scene: &Scene, size: Size<Pixels>, dark: bool) -> anyhow::Result<Self> {
        let text = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(theme::SANS));
        let mut cx = HeadlessAppContext::with_platform(text, Arc::new(ui::Assets), gpui_platform::current_headless_renderer);
        let host = Arc::new(host::Staged {
            inputs: vec![("alsa:headset", "Headset-Mikrofon"), ("alsa:usb", "USB-Mikrofon"), ("alsa:webcam", "Webcam")],
            outputs: vec![("alsa:headset-out", "Headset"), ("alsa:speakers", "Lautsprecher")],
            card: scene.card,
            installing: scene.installing,
        });
        let (call, happenings) = Call::staged(scene.stage.clone())?;
        let client = cx.update(|cx| {
            ui::init(cx);
            theme::force(dark, cx);
            cx.set_global(Marks::default());
            // GPUI's motions follow the wall clock, which differs from run to run.
            cx.set_reduce_motion(true);
            cx.new(|cx| Client::new(call, happenings, host, scene.settings.clone(), None, Some("Strg+Alt+M"), cx))
        });
        let window = cx.open_window(size, |window, cx| cx.new(|cx| Main::new(client, None, window, cx)))?.into();
        let mut harness = Self { cx, window, pointer: Point::default() };
        harness.settle();
        for step in Step::parse(scene.steps)? {
            harness.step(&step)?;
        }
        Ok(harness)
    }

    pub fn step(&mut self, step: &Step) -> anyhow::Result<()> {
        self.start(step)?;
        if !matches!(step, Step::Wait(_)) {
            self.settle();
        }
        Ok(())
    }

    pub fn film(&mut self, step: &Step, frames: u32, every: Duration) -> anyhow::Result<Vec<RgbaImage>> {
        self.cx.update(|cx| cx.set_reduce_motion(false));
        self.start(step)?;
        let mut shots = Vec::new();
        for _ in 0..frames {
            let drawn = std::time::Instant::now();
            shots.push(self.snap()?);
            std::thread::sleep(every.saturating_sub(drawn.elapsed()));
            self.cx.advance_clock(every);
        }
        Ok(shots)
    }

    fn start(&mut self, step: &Step) -> anyhow::Result<()> {
        match step {
            Step::Click(name) => {
                let at = self.find(name)?;
                self.move_to(at);
                self.input(PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position: at,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }));
                self.input(PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position: at,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }));
            }
            Step::Hover(name) => {
                let at = self.find(name)?;
                self.move_to(at);
            }
            Step::Type(text) => {
                for character in text.chars() {
                    let key = match character {
                        ' ' => "space".to_owned(),
                        character => character.to_string(),
                    };
                    let keystroke = Keystroke { key_char: Some(character.to_string()), ..keystroke(&key)? };
                    self.keystroke(keystroke)?;
                }
            }
            Step::Key(keys) => {
                for key in keys.split_whitespace() {
                    self.keystroke(keystroke(key)?)?;
                }
            }
            Step::Wait(duration) => self.cx.advance_clock(*duration),
        }
        self.draw()
    }

    pub fn snap(&mut self) -> anyhow::Result<RgbaImage> {
        self.draw()?;
        self.cx.capture_screenshot(self.window)
    }

    pub fn marks(&mut self) -> anyhow::Result<Vec<(SharedString, Bounds<Pixels>)>> {
        self.draw()?;
        Ok(self.cx.update(|cx| cx.global::<Marks>().0.clone()))
    }

    fn find(&mut self, name: &str) -> anyhow::Result<Point<Pixels>> {
        let (name, nth) = match name.rsplit_once('#') {
            Some((name, nth)) => (name, nth.parse::<usize>().with_context(|| format!("{name}#{nth}: a count from 1"))?),
            None => (name, 1),
        };
        let marks = self.marks()?;
        let found = marks.iter().filter(|(mark, _)| mark.as_ref() == name).nth(nth.saturating_sub(1));
        match found {
            Some((_, bounds)) => Ok(bounds.center()),
            None => {
                let names: Vec<&str> = marks.iter().map(|(name, _)| name.as_ref()).collect();
                bail!("nothing called {name} (#{nth}) is drawn; there are: {}", names.join(", "))
            }
        }
    }

    fn move_to(&mut self, at: Point<Pixels>) {
        self.pointer = at;
        self.input(PlatformInput::MouseMove(MouseMoveEvent { position: at, pressed_button: None, modifiers: Modifiers::default() }));
    }

    fn input(&mut self, event: PlatformInput) {
        // What the window did with it shows in what it draws next.
        let _ = self.cx.update_window(self.window, |_, window, cx| window.dispatch_event(event, cx));
    }

    fn keystroke(&mut self, keystroke: Keystroke) -> anyhow::Result<()> {
        self.cx.update_window(self.window, |_, window, cx| window.dispatch_keystroke(keystroke, cx))?;
        Ok(())
    }

    fn settle(&mut self) {
        self.cx.run_until_parked();
        for _ in 0..SETTLE.as_millis() / 50 {
            self.cx.advance_clock(Duration::from_millis(50));
            // A draw that fails shows when the next snap draws.
            let _ = self.draw();
        }
    }

    fn draw(&mut self) -> anyhow::Result<()> {
        self.cx.run_until_parked();
        self.cx.update(Marks::clear);
        self.cx.update_window(self.window, |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }
}
