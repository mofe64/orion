use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{OrionRuntimeError as Error, Result};

pub const ORION_LIGHT_PIXEL_COUNT: usize = 40;
pub const ORION_LIGHT_WIDTH: usize = 8;
pub const ORION_LIGHT_HEIGHT: usize = 5;
pub const ORION_WHITE_TEMPERATURE_K: u16 = 3000;
pub const ORION_LIGHT_GPIO_BCM: u8 = 12;
pub const PI5_NEOPIXEL_DEVICE_PATH: &str = "/dev/ws281x_pwm";
pub const LIGHTING_EFFECT_NAMES: [&str; 9] = [
    "warm_idle_breathe",
    "attentive_focus",
    "thinking_drift",
    "speaking_energy",
    "acknowledge_pulse",
    "curious_sweep",
    "delight_spark",
    "settle_glow",
    "off",
];

const NEOPIXEL_FREQUENCY_HZ: usize = 800_000;
const NEOPIXEL_COLOR_CHANNELS: usize = 4;
const NEOPIXEL_SYMBOLS_PER_BIT: usize = 3;
const NEOPIXEL_RESET_MICROSECONDS: usize = 55;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rgbw8 {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub white: u8,
}

impl Rgbw8 {
    pub const OFF: Self = Self {
        red: 0,
        green: 0,
        blue: 0,
        white: 0,
    };

    pub const fn new(red: u8, green: u8, blue: u8, white: u8) -> Self {
        Self {
            red,
            green,
            blue,
            white,
        }
    }

    pub fn interpolate(self, target: Self, progress: f64) -> Result<Self> {
        if !progress.is_finite() {
            return Err(Error::InvalidArgument(
                "Light transition progress must be finite.".into(),
            ));
        }
        let progress = progress.clamp(0.0, 1.0);
        Ok(Self {
            red: interpolate_channel(self.red, target.red, progress),
            green: interpolate_channel(self.green, target.green, progress),
            blue: interpolate_channel(self.blue, target.blue, progress),
            white: interpolate_channel(self.white, target.white, progress),
        })
    }
}

pub fn render_effect(name: &str, elapsed_seconds: f64, intensity: f64) -> Result<Vec<Rgbw8>> {
    render_effect_for_pixels(name, elapsed_seconds, intensity, ORION_LIGHT_PIXEL_COUNT)
}

pub fn render_effect_for_pixels(
    name: &str,
    elapsed_seconds: f64,
    intensity: f64,
    pixel_count: usize,
) -> Result<Vec<Rgbw8>> {
    if !elapsed_seconds.is_finite() || !intensity.is_finite() || !(0.0..=1.0).contains(&intensity) {
        return Err(Error::InvalidArgument(
            "Lighting effects require finite time and intensity from zero to one.".into(),
        ));
    }
    if !LIGHTING_EFFECT_NAMES.contains(&name) {
        return Err(Error::InvalidArgument(format!(
            "Unknown Orion lighting effect: {name}"
        )));
    }
    if name == "off" {
        return Ok(vec![Rgbw8::OFF; pixel_count]);
    }
    if pixel_count != 24 && pixel_count != ORION_LIGHT_PIXEL_COUNT {
        return Err(Error::InvalidArgument(
            "Effects require the v1 matrix or v2 ring layout.".into(),
        ));
    }
    let phase = elapsed_seconds.rem_euclid(4.) * std::f64::consts::TAU / 4.;
    Ok((0..pixel_count)
        .map(|index| {
            let angle = index as f64 * std::f64::consts::TAU / pixel_count as f64;
            let nx = (index % ORION_LIGHT_WIDTH) as f64 / (ORION_LIGHT_WIDTH - 1) as f64;
            let spatial = if pixel_count == 24 {
                angle
            } else {
                nx * std::f64::consts::TAU
            };
            let level = match name {
                "warm_idle_breathe" => 0.52 + 0.40 * phase.sin(),
                "attentive_focus" => {
                    let focus = if pixel_count == 24 {
                        0.5 + 0.5 * angle.cos()
                    } else {
                        1. - ((nx - 0.5).abs() * 1.7).min(1.)
                    };
                    0.12 + 0.80 * focus * (0.65 + 0.35 * phase.cos())
                }
                "thinking_drift" => {
                    let drift = if pixel_count == 24 {
                        0.5 + 0.5 * (angle - phase).cos()
                    } else {
                        1. - ((nx - (0.5 + 0.4 * phase.sin())).abs() * 2.5).min(1.)
                    };
                    0.12 + 0.80 * drift
                }
                "speaking_energy" => 0.14 + 0.80 * (0.5 + 0.5 * (phase * 4. - spatial).sin()),
                "acknowledge_pulse" => 0.16 + 0.82 * (0.5 + 0.5 * (phase * 2.).cos()).powi(4),
                "curious_sweep" => 0.10 + 0.86 * (0.5 + 0.5 * (spatial - phase * 2.).cos()).powi(4),
                "delight_spark" => {
                    0.14 + if (spatial * 5. + phase * 7.).sin() > 0.72 {
                        0.82
                    } else {
                        0.04
                    }
                }
                "settle_glow" => 0.18 + 0.76 * (0.5 + 0.5 * phase.cos()).powi(2),
                _ => unreachable!(),
            } * intensity;
            let warm = (level.clamp(0., 1.) * 255.).round() as u8;
            Rgbw8::new(
                warm / 5,
                warm / 10,
                warm / 40,
                (u16::from(warm) * 3 / 4) as u8,
            )
        })
        .collect())
}

fn interpolate_channel(start: u8, target: u8, progress: f64) -> u8 {
    (f64::from(start) + (f64::from(target) - f64::from(start)) * progress).round() as u8
}

pub trait LightingDevice {
    fn pixel_count(&self) -> usize;
    fn render(&mut self, pixels: &[Rgbw8]) -> Result<()>;

    fn render_effect(&mut self, name: &str, elapsed: f64, intensity: f64) -> Result<()> {
        self.render(&render_effect_for_pixels(
            name,
            elapsed,
            intensity,
            self.pixel_count(),
        )?)
    }

    fn render_uniform(&mut self, color: Rgbw8) -> Result<()> {
        self.render(&vec![color; self.pixel_count()])
    }

    fn clear(&mut self) -> Result<()> {
        self.render_uniform(Rgbw8::OFF)
    }
}

#[derive(Debug)]
pub struct RecordingLightingDevice {
    pixel_count: usize,
    frames: Vec<Vec<Rgbw8>>,
}

impl RecordingLightingDevice {
    pub fn new(pixel_count: usize) -> Result<Self> {
        if pixel_count == 0 {
            return Err(Error::InvalidArgument(
                "A lighting device must contain at least one pixel.".into(),
            ));
        }
        Ok(Self {
            pixel_count,
            frames: Vec::new(),
        })
    }

    pub fn orion() -> Self {
        Self {
            pixel_count: ORION_LIGHT_PIXEL_COUNT,
            frames: Vec::new(),
        }
    }

    pub fn frames(&self) -> &[Vec<Rgbw8>] {
        &self.frames
    }

    pub fn last_frame(&self) -> Option<&[Rgbw8]> {
        self.frames.last().map(Vec::as_slice)
    }
}

impl LightingDevice for RecordingLightingDevice {
    fn pixel_count(&self) -> usize {
        self.pixel_count
    }

    fn render(&mut self, pixels: &[Rgbw8]) -> Result<()> {
        if pixels.len() != self.pixel_count {
            return Err(Error::InvalidArgument(format!(
                "Lighting frame contains {} pixels; device requires {}.",
                pixels.len(),
                self.pixel_count
            )));
        }
        self.frames.push(pixels.to_vec());
        Ok(())
    }
}

/// Raspberry Pi 5 lighting output backed by the RP1 PWM character device from
/// the official `rpi_ws281x` Pi 5 kernel module.
///
/// Orion's shield is an SK6812-compatible RGBW matrix. Its wire order is GRBW,
/// so callers continue to use logical RGBW values while this adapter performs
/// the physical channel ordering and NeoPixel symbol encoding.
#[derive(Debug)]
pub struct Pi5NeoPixelDevice {
    file: File,
    path: PathBuf,
    pixel_count: usize,
}

impl Pi5NeoPixelDevice {
    pub fn open_orion() -> Result<Self> {
        Self::open(PI5_NEOPIXEL_DEVICE_PATH, ORION_LIGHT_PIXEL_COUNT)
    }

    pub fn open(path: impl AsRef<Path>, pixel_count: usize) -> Result<Self> {
        if pixel_count == 0 {
            return Err(Error::InvalidArgument(
                "A lighting device must contain at least one pixel.".into(),
            ));
        }
        let path = path.as_ref();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| {
                Error::Runtime(format!(
                    "Could not open Pi 5 NeoPixel device '{}': {error}. Load the rp1_ws281x_pwm module first.",
                    path.display()
                ))
            })?;
        Ok(Self {
            file,
            path: path.to_owned(),
            pixel_count,
        })
    }

    pub fn device_path(&self) -> &Path {
        &self.path
    }
}

impl LightingDevice for Pi5NeoPixelDevice {
    fn pixel_count(&self) -> usize {
        self.pixel_count
    }

    fn render(&mut self, pixels: &[Rgbw8]) -> Result<()> {
        if pixels.len() != self.pixel_count {
            return Err(Error::InvalidArgument(format!(
                "Lighting frame contains {} pixels; device requires {}.",
                pixels.len(),
                self.pixel_count
            )));
        }
        let frame = encode_pi5_grbw_frame(pixels);
        self.file.write_all(&frame).map_err(|error| {
            Error::Runtime(format!(
                "Could not render RGBW frame through '{}': {error}",
                self.path.display()
            ))
        })
    }
}

fn encode_pi5_grbw_frame(pixels: &[Rgbw8]) -> Vec<u8> {
    // Match PCM_BYTE_COUNT in the official Pi 5 rpi_ws281x branch. The RP1
    // kernel driver accepts one PWM channel, so no two-channel interleaving is
    // present in the bytes written to /dev/ws281x_pwm.
    let data_bits = pixels.len() * NEOPIXEL_COLOR_CHANNELS * 8 * NEOPIXEL_SYMBOLS_PER_BIT;
    let reset_bits = NEOPIXEL_RESET_MICROSECONDS * (NEOPIXEL_FREQUENCY_HZ * 3) / 1_000_000;
    let frame_bytes = ((((data_bits + reset_bits) >> 3) & !0x7) + 4) + 4;
    let mut natural = Vec::with_capacity(pixels.len() * NEOPIXEL_COLOR_CHANNELS * 3);

    for pixel in pixels {
        // Adafruit product 2864 uses the SK6812 GRBW byte order.
        for channel in [pixel.green, pixel.red, pixel.blue, pixel.white] {
            natural.extend_from_slice(&encode_neopixel_byte(channel));
        }
    }

    let mut frame = vec![0_u8; frame_bytes];
    for (source, target) in natural.chunks_exact(4).zip(frame.chunks_exact_mut(4)) {
        target.copy_from_slice(&[source[3], source[2], source[1], source[0]]);
    }
    frame
}

fn encode_neopixel_byte(value: u8) -> [u8; 3] {
    let mut encoded = 0_u32;
    for bit in (0..8).rev() {
        // One NeoPixel data bit is represented by three PWM symbols: 100 for
        // zero and 110 for one.
        encoded = (encoded << 3)
            | if value & (1 << bit) == 0 {
                0b100
            } else {
                0b110
            };
    }
    [
        ((encoded >> 16) & 0xff) as u8,
        ((encoded >> 8) & 0xff) as u8,
        (encoded & 0xff) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_all_four_channels() {
        let start = Rgbw8::new(0, 10, 20, 30);
        let target = Rgbw8::new(100, 110, 120, 130);
        assert_eq!(
            start.interpolate(target, 0.25).unwrap(),
            Rgbw8::new(25, 35, 45, 55)
        );
        assert_eq!(start.interpolate(target, -1.0).unwrap(), start);
        assert_eq!(start.interpolate(target, 2.0).unwrap(), target);
        assert!(start.interpolate(target, f64::NAN).is_err());
    }

    #[test]
    fn records_only_frames_matching_the_device() {
        let mut device = RecordingLightingDevice::new(2).unwrap();
        device.render_uniform(Rgbw8::new(1, 2, 3, 4)).unwrap();
        assert_eq!(device.last_frame().unwrap(), &[Rgbw8::new(1, 2, 3, 4); 2]);
        assert!(device.render(&[Rgbw8::OFF]).is_err());
    }

    #[test]
    fn studio_effect_table_matches_the_lamp_at_every_sample() {
        let mut layouts = serde_json::Map::new();
        for count in [24, 40] {
            let mut effects = serde_json::Map::new();
            for name in LIGHTING_EFFECT_NAMES {
                let frames: Vec<Vec<u8>> = (0..200)
                    .map(|tick| {
                        render_effect_for_pixels(name, tick as f64 / 50., 1., count)
                            .unwrap()
                            .into_iter()
                            .map(|p| p.white)
                            .collect()
                    })
                    .collect();
                effects.insert(name.into(), serde_json::json!(frames));
            }
            layouts.insert(count.to_string(), serde_json::json!(effects));
        }
        let mut examples = Vec::new();
        for count in [24, 40] {
            for name in LIGHTING_EFFECT_NAMES {
                let lamp = crate::expression::lamp::LampProgram::default().updated_at(
                    serde_json::from_value(serde_json::json!({"effect":name,"brightness":0.65,"colors":[[40,20,4,210],[255,0,0,0]]})).unwrap(), 10000.).unwrap();
                for tick in [0, 17, 81, 149, 199] {
                    let time = tick as f64 / 50.;
                    let pixels: Vec<_> = lamp
                        .render_for_pixels(10000. + time, count)
                        .unwrap()
                        .into_iter()
                        .map(|p| [p.red, p.green, p.blue, p.white])
                        .collect();
                    examples.push(serde_json::json!({"count":count,"effect":name,"time":time,"pixels":pixels}));
                }
            }
        }
        let generated =
            serde_json::json!({"fps":50,"period":4,"layouts":layouts,"examples":examples});
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../orion_studio/src/generated/lampEffects.json");
        if std::env::var_os("ORION_GENERATE_LAMP_TABLE").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string(&generated).unwrap() + "\n").unwrap();
        }
        let saved: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&path).expect("Regenerate the Studio lamp table"),
        )
        .unwrap();
        assert_eq!(
            saved, generated,
            "Regenerate with ORION_GENERATE_LAMP_TABLE=1 cargo test --manifest-path runtime/Cargo.toml studio_effect_table"
        );
        for count in [24, 40] {
            for name in LIGHTING_EFFECT_NAMES
                .into_iter()
                .filter(|name| *name != "off")
            {
                let frames: Vec<_> = (0..200)
                    .map(|tick| {
                        render_effect_for_pixels(name, tick as f64 / 50., 1., count).unwrap()
                    })
                    .collect();
                let contrast = (0..count)
                    .map(|pixel| {
                        let minimum = frames.iter().map(|f| f[pixel].white).min().unwrap();
                        let maximum = frames.iter().map(|f| f[pixel].white).max().unwrap();
                        maximum - minimum
                    })
                    .max()
                    .unwrap();
                assert!(
                    contrast >= 60,
                    "{name} must have visible animation contrast on {count} pixels"
                );
            }
        }
    }

    #[test]
    fn every_named_effect_renders_exactly_the_orion_matrix() {
        for name in LIGHTING_EFFECT_NAMES {
            let frame = render_effect(name, 0.25, 0.8).unwrap();
            assert_eq!(frame.len(), ORION_LIGHT_PIXEL_COUNT, "{name}");
        }
        assert!(render_effect("rainbow_party", 0.0, 1.0).is_err());
    }

    #[test]
    fn every_named_effect_renders_the_ring_through_the_device_boundary() {
        let mut ring = RecordingLightingDevice::new(24).unwrap();
        for name in LIGHTING_EFFECT_NAMES {
            ring.render_effect(name, 0.25, 0.8).unwrap();
            assert_eq!(ring.last_frame().unwrap().len(), 24, "{name}");
        }
        assert!(ring.last_frame().unwrap().iter().all(|p| *p == Rgbw8::OFF));
        assert_eq!(encode_pi5_grbw_frame(&vec![Rgbw8::OFF; 24]).len(), 312);
        assert!(
            encode_pi5_grbw_frame(&vec![Rgbw8::OFF; 24])[288..]
                .iter()
                .all(|v| *v == 0)
        );
    }

    #[test]
    fn encodes_neopixel_zero_and_one_symbols() {
        assert_eq!(encode_neopixel_byte(0x00), [0x92, 0x49, 0x24]);
        assert_eq!(encode_neopixel_byte(0x01), [0x92, 0x49, 0x26]);
        assert_eq!(encode_neopixel_byte(0xff), [0xdb, 0x6d, 0xb6]);
    }

    #[test]
    fn encodes_orion_frames_as_grbw_with_reset_padding() {
        let pixels = vec![Rgbw8::OFF; ORION_LIGHT_PIXEL_COUNT];
        let frame = encode_pi5_grbw_frame(&pixels);
        assert_eq!(frame.len(), 504);
        // Four natural encoded bytes are reversed for the RP1 PWM word.
        assert_eq!(&frame[..4], &[0x92, 0x24, 0x49, 0x92]);
        assert!(frame[480..].iter().all(|byte| *byte == 0));

        let mut first_pixel = pixels;
        first_pixel[0] = Rgbw8::new(0x00, 0x01, 0x00, 0x00);
        let frame = encode_pi5_grbw_frame(&first_pixel);
        assert_eq!(&frame[..4], &[0x92, 0x26, 0x49, 0x92]);
    }

    #[test]
    fn writes_an_encoded_frame_to_the_configured_device() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ws281x_pwm");
        File::create(&path).unwrap();
        let mut device = Pi5NeoPixelDevice::open(&path, 1).unwrap();
        device.render(&[Rgbw8::new(1, 2, 3, 4)]).unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(bytes, encode_pi5_grbw_frame(&[Rgbw8::new(1, 2, 3, 4)]));
    }
}
