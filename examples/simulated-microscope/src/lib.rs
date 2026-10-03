//! A simulated microscope: the capstone example.
//!
//! Three Things, composed with slots:
//!
//! - [`Stage`] moves the slide in x, y and z at a speed that is a setting,
//!   one small step at a time, so moves take time and can be cancelled. It
//!   announces each arrival with an event.
//! - [`Camera`] looks at the slide through the stage (a slot). A thread
//!   renders what it sees into an MJPEG preview: a field of cells, blurred
//!   by the distance from the focal plane. `capture` returns a JPEG Blob,
//!   `sharpness` measures the image, and `exposure` is a setting.
//! - [`Autofocus`] sweeps the stage's z through the camera's sharpness (two
//!   slots), moves to the sharpest position, and returns the focus curve as
//!   an array.
//!
//! [`registry`] names them as a configuration file does, and
//! [`CONFIG`] is such a file.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use teta_wot::blob::Jpeg;
use teta_wot::ndarray::{Array2, Ix2};
use teta_wot::prelude::*;
use teta_wot::server::ThingRegistry;

/// The z position at which the slide is in focus.
pub const FOCUS_Z: i64 = 250;

/// How far out of focus the image is fully blurred, in steps.
const DEPTH_OF_FIELD: f64 = 40.0;

/// The preview's size, in pixels.
const WIDTH: usize = 320;
const HEIGHT: usize = 240;

/// Stage steps per pixel.
const STEPS_PER_PIXEL: i64 = 2;

/// A configuration file serving the microscope, with the Things'
/// import strings and `./settings/microscope` as the settings folder.
pub const CONFIG: &str = include_str!("../microscope.json");

// ANCHOR: registry
/// The Thing types, under the import strings of [`CONFIG`].
pub fn registry() -> ThingRegistry {
    ThingRegistry::new()
        .register::<Stage>("microscope.stage:Stage")
        .register::<Camera>("microscope.camera:Camera")
        .register::<Autofocus>("microscope.autofocus:Autofocus")
}
// ANCHOR_END: registry

// ---- The stage ---------------------------------------------------------------

/// A position of the stage, in steps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Position {
    /// Left to right.
    pub x: i64,
    /// Front to back.
    pub y: i64,
    /// Up and down: the focus.
    pub z: i64,
}

// ANCHOR: stage
/// A motorised stage, simulated.
///
/// It moves the slide under the objective in x and y, and the focus in z.
#[derive(Thing)]
pub struct Stage {
    /// Where the stage is, in steps.
    #[property(readonly, default = Position::default())]
    position: Prop<Position>,
    /// How fast the stage moves, in steps per second.
    #[setting(default = 2000.0, gt = 0, le = 100_000)]
    speed: Prop<f64>,
    /// The stage arrived where it was sent.
    #[event]
    arrived: Event<Position>,
}
// ANCHOR_END: stage

impl Stage {
    /// The stage's position now.
    pub fn now(&self) -> Position {
        self.position.get()
    }
}

#[thing_impl]
impl Stage {
    /// Move to a position.
    ///
    /// The stage moves in small steps at its speed, and stops where it is if
    /// the move is cancelled.
    #[action(global_lock = false)]
    async fn move_to(&self, x: i64, y: i64, z: i64) -> Result<Position, ActionError> {
        self.travel(Position { x, y, z }).await
    }

    /// Move by a distance from where the stage is.
    #[action(global_lock = false)]
    async fn move_by(
        &self,
        #[param(default = 0)] dx: i64,
        #[param(default = 0)] dy: i64,
        #[param(default = 0)] dz: i64,
    ) -> Result<Position, ActionError> {
        let here = self.now();
        self.travel(Position {
            x: here.x + dx,
            y: here.y + dy,
            z: here.z + dz,
        })
        .await
    }

    /// Return to the origin.
    #[action(global_lock = false)]
    async fn home(&self) -> Result<Position, ActionError> {
        self.travel(Position::default()).await
    }
}

impl Stage {
    async fn travel(&self, target: Position) -> Result<Position, ActionError> {
        const TICK: Duration = Duration::from_millis(20);
        let start = self.now();
        let distance = [target.x - start.x, target.y - start.y, target.z - start.z]
            .iter()
            .map(|d| d.abs())
            .max()
            .unwrap_or(0);
        let steps_per_tick = (self.speed.get() * TICK.as_secs_f64()).max(1.0);
        let ticks = (distance as f64 / steps_per_tick).ceil() as i64;
        tracing::info!("moving from {start:?} to {target:?} in {ticks} ticks");
        for tick in 1..=ticks {
            cancellable_sleep(TICK).await?;
            let along = |from: i64, to: i64| from + (to - from) * tick / ticks;
            self.position.set(Position {
                x: along(start.x, target.x),
                y: along(start.y, target.y),
                z: along(start.z, target.z),
            })?;
        }
        self.position.set(target)?;
        self.arrived.emit(target);
        Ok(target)
    }
}

// ---- The camera --------------------------------------------------------------

/// What a capture was.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Capture {
    /// Where the stage was.
    pub position: Position,
    /// The exposure, in milliseconds.
    pub exposure: f64,
    /// The size of the JPEG, in bytes.
    pub bytes: usize,
}

// ANCHOR: camera
/// A camera looking at the slide on the stage, simulated.
///
/// It sees a field of cells, sharp at the focal plane and blurred away
/// from it, brighter with a longer exposure.
#[derive(Thing)]
pub struct Camera {
    /// The stage the slide is on.
    #[slot]
    stage: Slot<Stage>,
    /// The live preview, as MJPEG.
    #[stream]
    preview: MjpegStream,
    /// The exposure time, in milliseconds.
    #[setting(default = 20.0, gt = 0, le = 1000)]
    exposure: Prop<f64>,
    /// Frames per second of the preview.
    #[property(default = 10.0, gt = 0, le = 30)]
    fps: Prop<f64>,
    /// An image was captured.
    #[event]
    captured: Event<Capture>,
    running: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}
// ANCHOR_END: camera

#[thing_impl]
impl Camera {
    // ANCHOR: preview
    /// Starts the thread that feeds the preview.
    #[on_start]
    async fn start_preview(&self) -> anyhow::Result<()> {
        let stage = Arc::clone(ThingRef::thing(&self.stage));
        let (preview, exposure, fps) = (
            self.preview.clone(),
            self.exposure.subscribe(),
            self.fps.subscribe(),
        );
        let running = Arc::clone(&self.running);
        running.store(true, Ordering::SeqCst);
        let thread = std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || {
                while running.load(Ordering::SeqCst) {
                    let image = render(stage.now(), *exposure.borrow());
                    match encode(&image) {
                        Ok(jpeg) => {
                            let _ = preview.add_frame(jpeg);
                        }
                        Err(error) => tracing::error!("the camera failed: {error}"),
                    }
                    std::thread::sleep(Duration::from_secs_f64(1.0 / *fps.borrow()));
                }
            })?;
        *self.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(thread);
        Ok(())
    }

    /// Stops the preview's thread, and ends the clients' streams.
    #[on_stop]
    async fn stop_preview(&self) {
        self.running.store(false, Ordering::SeqCst);
        let thread = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(thread) = thread {
            let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        }
        self.preview.stop();
    }
    // ANCHOR_END: preview

    /// How sharp the image is: the mean squared difference between
    /// neighbouring pixels, which blurring reduces.
    #[property]
    async fn sharpness(&self) -> f64 {
        self.measure()
    }

    // ANCHOR: capture
    /// Capture an image, as a JPEG.
    #[action]
    async fn capture(&self) -> Result<Blob<Jpeg>, ActionError> {
        let (position, exposure) = (self.stage_now(), self.exposure.get());
        let jpeg = encode(&render(position, exposure))?;
        self.captured.emit(Capture {
            position,
            exposure,
            bytes: jpeg.len(),
        });
        Ok(Blob::from_bytes(jpeg))
    }
    // ANCHOR_END: capture
}

impl Camera {
    fn stage_now(&self) -> Position {
        ThingRef::thing(&self.stage).now()
    }

    /// The sharpness of the image the camera sees now.
    pub fn measure(&self) -> f64 {
        sharpness(&render(self.stage_now(), self.exposure.get()))
    }
}

/// A greyscale image, row by row.
struct Image(Vec<f64>);

/// What the camera sees with the stage at `position`: cells on a slide, in
/// a pattern that repeats every 1024 pixels, blurred by the distance from
/// the focal plane.
fn render(position: Position, exposure: f64) -> Image {
    let (left, top) = (position.x / STEPS_PER_PIXEL, position.y / STEPS_PER_PIXEL);
    let mut pixels = vec![0.0; WIDTH * HEIGHT];
    for (row, line) in pixels.chunks_mut(WIDTH).enumerate() {
        for (column, pixel) in line.iter_mut().enumerate() {
            *pixel = slide(left + column as i64, top + row as i64);
        }
    }
    let radius = ((position.z - FOCUS_Z).abs() as f64 / DEPTH_OF_FIELD * 3.0).round() as usize;
    for _ in 0..3 {
        box_blur(&mut pixels, radius);
    }
    let gain = (exposure / 20.0).clamp(0.05, 10.0);
    Image(pixels.into_iter().map(|p| (p * gain).min(1.0)).collect())
}

/// The slide at a pixel: cells (bright discs with a dark nucleus) on a dim
/// background, one per 32-pixel square, placed by a hash.
fn slide(x: i64, y: i64) -> f64 {
    let (cell_x, cell_y) = (x.div_euclid(32), y.div_euclid(32));
    let mut brightness: f64 = 0.1;
    for (dx, dy) in [
        (0, 0),
        (-1, 0),
        (0, -1),
        (-1, -1),
        (1, 0),
        (0, 1),
        (1, 1),
        (1, -1),
        (-1, 1),
    ] {
        let (cx, cy) = (cell_x + dx, cell_y + dy);
        let hash = hash(cx.rem_euclid(32), cy.rem_euclid(32));
        let centre_x = cx * 32 + 8 + (hash % 16) as i64;
        let centre_y = cy * 32 + 8 + ((hash >> 8) % 16) as i64;
        let radius = 6.0 + ((hash >> 16) % 6) as f64;
        let distance = (((x - centre_x).pow(2) + (y - centre_y).pow(2)) as f64).sqrt();
        if distance < radius / 3.0 {
            brightness = brightness.max(0.35);
        } else if distance < radius {
            brightness = brightness.max(0.8);
        }
    }
    brightness
}

fn hash(x: i64, y: i64) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    h ^= h >> 29;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^ (h >> 32)
}

/// A box blur of `radius` pixels, horizontally then vertically, with a
/// running sum so that it costs the same whatever the radius.
fn box_blur(pixels: &mut [f64], radius: usize) {
    if radius == 0 {
        return;
    }
    let mut out = vec![0.0; pixels.len()];
    for (length, stride, lines, step) in [(WIDTH, 1, HEIGHT, WIDTH), (HEIGHT, WIDTH, WIDTH, 1)] {
        for line in 0..lines {
            let at = |i: usize| line * step + i * stride;
            let mut sum: f64 = (0..=radius.min(length - 1)).map(|j| pixels[at(j)]).sum();
            for i in 0..length {
                let (from, to) = (i.saturating_sub(radius), (i + radius).min(length - 1));
                out[at(i)] = sum / (to - from + 1) as f64;
                // Slide the window: add the next pixel, drop the oldest.
                if i + radius + 1 < length {
                    sum += pixels[at(i + radius + 1)];
                }
                if i >= radius {
                    sum -= pixels[at(i - radius)];
                }
            }
        }
        pixels.copy_from_slice(&out);
    }
}

fn sharpness(image: &Image) -> f64 {
    let pixels = &image.0;
    let mut total = 0.0;
    for row in 0..HEIGHT {
        for column in 1..WIDTH {
            let i = row * WIDTH + column;
            total += (pixels[i] - pixels[i - 1]).powi(2);
        }
    }
    total / (HEIGHT * (WIDTH - 1)) as f64 * 1000.0
}

fn encode(image: &Image) -> anyhow::Result<Vec<u8>> {
    let grey: Vec<u8> = image.0.iter().map(|p| (p * 255.0) as u8).collect();
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, 85).encode(
        &grey,
        WIDTH as u16,
        HEIGHT as u16,
        jpeg_encoder::ColorType::Luma,
    )?;
    Ok(jpeg)
}

// ---- The autofocus -----------------------------------------------------------

/// What an autofocus found.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Focus {
    /// The sharpest z.
    pub z: i64,
    /// Its sharpness.
    pub sharpness: f64,
    /// The sweep: one row per position, `[z, sharpness]`.
    pub curve: NdArray<f64, Ix2>,
}

// ANCHOR: autofocus
/// Focuses the camera by moving the stage.
#[derive(Thing)]
pub struct Autofocus {
    /// The stage that moves the focus.
    #[slot]
    stage: Slot<Stage>,
    /// The camera whose sharpness is measured.
    #[slot]
    camera: Slot<Camera>,
    /// The last focus found, if any.
    #[property(readonly, default = None)]
    last_focus: Prop<Option<i64>>,
    /// A focus was found.
    #[event]
    focused: Event<i64>,
}

#[thing_impl]
impl Autofocus {
    /// Find the focus.
    ///
    /// The stage sweeps z over `range` steps around where it is, in `steps`
    /// moves; the camera measures the sharpness at each, and the stage
    /// returns to the sharpest.
    #[action]
    async fn run(
        &self,
        ctx: ActionCtx,
        #[param(default = 1000)] range: i64,
        #[param(default = 21)] steps: i64,
    ) -> Result<Focus, ActionError> {
        let steps = steps.clamp(2, 1000);
        let here = ThingRef::thing(&self.stage).now();
        let mut curve = Array2::zeros((steps as usize, 2));
        let mut best = (f64::MIN, here.z);
        for i in 0..steps {
            ctx.check_cancelled()?;
            let z = here.z - range / 2 + range * i / (steps - 1);
            self.stage.move_to(here.x, here.y, z).await?;
            let sharpness = ThingRef::thing(&self.camera).measure();
            tracing::info!("z = {z}: sharpness {sharpness:.3}");
            curve[[i as usize, 0]] = z as f64;
            curve[[i as usize, 1]] = sharpness;
            if sharpness > best.0 {
                best = (sharpness, z);
            }
        }
        self.stage.move_to(here.x, here.y, best.1).await?;
        self.last_focus.set(Some(best.1))?;
        self.focused.emit(best.1);
        tracing::info!("in focus at z = {}", best.1);
        Ok(Focus {
            z: best.1,
            sharpness: best.0,
            curve: NdArray(curve),
        })
    }
}
// ANCHOR_END: autofocus
