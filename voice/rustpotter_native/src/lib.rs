use pyo3::{exceptions::PyRuntimeError, prelude::*};
use rustpotter::{Rustpotter, RustpotterConfig, SampleFormat};
use std::sync::Mutex;

#[pyclass]
struct Detector {
    detector: Mutex<Rustpotter>,
    pending: Vec<u8>,
    frame_bytes: usize,
}

#[pymethods]
impl Detector {
    #[new]
    fn new(model_path: &str, threshold: f32) -> PyResult<Self> {
        let mut config = RustpotterConfig::default();
        config.fmt.sample_format = SampleFormat::I16;
        config.detector.avg_threshold = 0.0;
        config.detector.threshold = threshold;
        config.detector.min_scores = 3;
        config.detector.eager = true;

        let mut detector = Rustpotter::new(&config).map_err(PyRuntimeError::new_err)?;
        detector
            .add_wakeword_from_file("hey_orion", model_path)
            .map_err(PyRuntimeError::new_err)?;
        detector.update_detector_config(&config.detector);
        let frame_bytes = detector.get_bytes_per_frame();

        Ok(Self {
            detector: Mutex::new(detector),
            pending: Vec::with_capacity(frame_bytes * 2),
            frame_bytes,
        })
    }

    fn process_pcm16(&mut self, pcm: &[u8]) -> Option<(String, f32)> {
        self.pending.extend_from_slice(pcm);
        while self.pending.len() >= self.frame_bytes {
            let frame: Vec<u8> = self.pending.drain(..self.frame_bytes).collect();
            let detection = self
                .detector
                .get_mut()
                .expect("Rustpotter detector lock was poisoned")
                .process_bytes(&frame);
            if let Some(detection) = detection {
                return Some((detection.name, detection.score));
            }
        }
        None
    }

    fn reset(&mut self) {
        self.pending.clear();
        self.detector
            .get_mut()
            .expect("Rustpotter detector lock was poisoned")
            .reset();
    }
}

#[pymodule]
fn orion_rustpotter(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Detector>()
}
