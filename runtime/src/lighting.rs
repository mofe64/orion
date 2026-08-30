use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub const ORION_LIGHT_PIXEL_COUNT: usize = 40;
pub const ORION_LIGHT_WIDTH: usize = 8;
pub const ORION_LIGHT_HEIGHT: usize = 5;
pub const ORION_WHITE_TEMPERATURE_K: u16 = 3000;
pub const ORION_LIGHT_GPIO_BCM: u8 = 12;
pub const PI5_NEOPIXEL_DEVICE_PATH: &str = "/dev/ws281x_pwm";

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

fn interpolate_channel(start: u8, target: u8, progress: f64) -> u8 {
    (f64::from(start) + (f64::from(target) - f64::from(start)) * progress).round() as u8
}

pub trait LightingDevice {
    fn pixel_count(&self) -> usize;
    fn render(&mut self, pixels: &[Rgbw8]) -> Result<()>;

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
