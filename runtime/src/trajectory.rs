use crate::pose::JointPositions;
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct JointTrajectory {
    name: String,
    start: JointPositions,
    target: JointPositions,
    duration_seconds: f64,
}

impl JointTrajectory {
    pub fn new(
        name: impl Into<String>,
        start: JointPositions,
        target: JointPositions,
        duration_seconds: f64,
    ) -> Result<Self> {
        let name = name.into();
        if name.is_empty() || !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err(Error::InvalidArgument(
                "Trajectory name and positive finite duration are required.".into(),
            ));
        }
        if start.is_empty() || start.len() != target.len() {
            return Err(Error::InvalidArgument(
                "Trajectory start and target joints must match.".into(),
            ));
        }
        for (joint_name, start_value) in &start {
            let Some(target_value) = target.get(joint_name) else {
                return Err(Error::InvalidArgument(
                    "Trajectory start and target joints must match and be finite.".into(),
                ));
            };
            if !start_value.is_finite() || !target_value.is_finite() {
                return Err(Error::InvalidArgument(
                    "Trajectory start and target joints must match and be finite.".into(),
                ));
            }
        }
        Ok(Self {
            name,
            start,
            target,
            duration_seconds,
        })
    }

    pub fn sample(&self, elapsed_seconds: f64) -> Result<JointPositions> {
        let phase = self.progress(elapsed_seconds)?;
        let phase2 = phase * phase;
        let phase3 = phase2 * phase;
        let blend = phase3 * (10.0 + phase * (-15.0 + 6.0 * phase));
        Ok(self
            .start
            .iter()
            .map(|(joint_name, start)| {
                let target = self.target[joint_name];
                (joint_name.clone(), start + (target - start) * blend)
            })
            .collect())
    }

    pub fn progress(&self, elapsed_seconds: f64) -> Result<f64> {
        if !elapsed_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "Trajectory elapsed time must be finite.".into(),
            ));
        }
        Ok((elapsed_seconds / self.duration_seconds).clamp(0.0, 1.0))
    }

    pub fn complete(&self, elapsed_seconds: f64) -> Result<bool> {
        Ok(self.progress(elapsed_seconds)? >= 1.0)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn positions(values: &[(&str, f64)]) -> JointPositions {
        values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect()
    }

    #[test]
    fn uses_quintic_endpoints_and_midpoint() {
        let trajectory = JointTrajectory::new(
            "test",
            positions(&[("a", 0.0), ("b", 1.0)]),
            positions(&[("a", 1.0), ("b", -1.0)]),
            2.0,
        )
        .unwrap();

        assert_eq!(trajectory.sample(0.0).unwrap()["a"], 0.0);
        assert_eq!(trajectory.sample(1.0).unwrap()["a"], 0.5);
        assert_eq!(trajectory.sample(1.0).unwrap()["b"], 0.0);
        assert_eq!(trajectory.sample(2.0).unwrap()["a"], 1.0);
        assert!(trajectory.complete(2.0).unwrap());
    }

    #[test]
    fn rejects_mismatched_targets_and_invalid_duration() {
        assert!(
            JointTrajectory::new(
                "bad",
                positions(&[("a", 0.0)]),
                positions(&[("b", 1.0)]),
                1.0
            )
            .is_err()
        );
        assert!(
            JointTrajectory::new(
                "bad",
                positions(&[("a", 0.0)]),
                positions(&[("a", 1.0)]),
                0.0
            )
            .is_err()
        );
    }
}
